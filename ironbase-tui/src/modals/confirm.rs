//! Confirm dialog modal

use super::render_modal_frame_rows;
use crate::i18n::t;
use crate::theme::Theme;
use crate::widgets::{choice, truncate_width};
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

/// Lines `text` takes wrapped at word boundaries into `width` columns (a
/// word wider than a line is broken), as the dialog's paragraph wraps it
fn wrapped_line_count(text: &str, width: u16) -> usize {
    use unicode_width::UnicodeWidthStr;
    let width = usize::from(width.max(1));
    text.lines()
        .map(|line| {
            let mut lines = 1;
            let mut used = 0;
            for word in line.split(' ') {
                let w = UnicodeWidthStr::width(word);
                let gap = usize::from(used > 0);
                if used + gap + w <= width {
                    used += gap + w;
                } else if w <= width {
                    lines += 1;
                    used = w;
                } else {
                    // Broken over lines: what is left of it starts the next
                    lines += usize::from(used > 0) + (w - 1) / width;
                    used = (w - 1) % width + 1;
                }
            }
            lines
        })
        .sum::<usize>()
        .max(1)
}

/// Render the confirm dialog
pub fn render(frame: &mut Frame, area: Rect, state: &ConfirmState, theme: &Theme) {
    // As high as the message needs (wrapped), at least 30% of the screen:
    // a fixed height cut a long message off
    const WIDTH_PCT: u16 = 50;
    let inner_width = (area.width * WIDTH_PCT / 100).saturating_sub(2);
    let needed = wrapped_line_count(&state.message, inner_width) as u16 + 2 + 2;
    let height = needed.max(area.height * 30 / 100);
    let inner = render_modal_frame_rows(frame, area, &state.title, theme, WIDTH_PCT, height);

    // Split into message and buttons
    let chunks = Layout::vertical([
        Constraint::Min(3),    // Message
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Buttons
    ])
    .split(inner);

    // Message
    // Wrapped: a line wider than the dialog (a long collection name, the
    // document preview) is not cut off
    let msg = Paragraph::new(state.message.clone())
        .style(Style::default().fg(theme.fg))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
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
    use super::wrapped_line_count;
    use ratatui::{backend::TestBackend, widgets::Paragraph, widgets::Wrap, Terminal};

    /// The estimate matches the lines ratatui's wrapping draws
    #[test]
    fn test_wrapped_line_count_matches_the_paragraph() {
        for (text, width) in [
            (
                "Biztosan törlöd a dokumentumot?\n\n{\"_id\":1,\"cím\":\"Az ember tragédiája\"}",
                20,
            ),
            ("rövid", 20),
            ("egy két három négy öt hat hét nyolc", 10),
            ("aaaaaaaaaaaaaaaaaaaaaaaaa b", 10),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            terminal
                .draw(|f| {
                    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), f.area())
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let last_drawn = (0..30)
                .rev()
                .find(|&y| (0..width).any(|x| buffer[(x, y)].symbol() != " "))
                .map_or(0, |y| y as usize + 1);
            assert_eq!(wrapped_line_count(text, width), last_drawn, "{text:?}");
        }
    }
}
