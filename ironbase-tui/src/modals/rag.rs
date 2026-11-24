//! RAG & Embedding modal - search, import, collections, models, jobs

use super::render_modal_frame;
use crate::i18n::t;
use crate::state::rag::{RagState, RagTab};
use crate::theme::Theme;
use crate::widgets::text_area::plain;
use ironbase_mcp_types::responses::{
    AutoEmbedStatusResponse, EmbedCacheStatsResponse, EmbedTextResponse, JobInfo, JobStatus,
    JobType, ModelInfo, RagCollectionStatsResponse, RagDocumentImportResponse,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};

/// Render the RAG & Embedding modal
pub fn render(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, t().rag_title(), theme, 85, 90);

    let chunks = Layout::vertical([
        Constraint::Length(3), // Tabs
        Constraint::Min(10),   // Tab content
        Constraint::Length(1), // Status/error
    ])
    .split(inner);

    render_tabs(frame, chunks[0], state, theme);

    match state.active_tab {
        RagTab::Search => render_search_tab(frame, chunks[1], state, theme),
        RagTab::Import => render_import_tab(frame, chunks[1], state, theme),
        RagTab::Collections => render_collections_tab(frame, chunks[1], state, theme),
        RagTab::Models => render_models_tab(frame, chunks[1], state, theme),
        RagTab::Jobs => render_jobs_tab(frame, chunks[1], state, theme),
    }

    render_status(frame, chunks[2], state, theme);
}

fn render_tabs(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let titles = vec![
        t().key_search(),
        t().tab_import(),
        t().label_collections(),
        t().tab_models(),
        t().tab_jobs(),
    ];
    let selected = match state.active_tab {
        RagTab::Search => 0,
        RagTab::Import => 1,
        RagTab::Collections => 2,
        RagTab::Models => 3,
        RagTab::Jobs => 4,
    };

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(selected)
        .style(Style::default().fg(theme.muted))
        .highlight_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

// ==================== Search tab ====================

fn render_search_tab(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Query input
        Constraint::Length(3), // Options row
        Constraint::Min(4),    // Results
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_tabs())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_search())),
        Span::styled("[↑/↓]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Query input
    let query_style = if state.search_focus == 0 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let query_block = Block::default()
        .title(format!(" {} ", t().label_query()))
        .borders(Borders::ALL)
        .border_style(query_style);
    let inner = query_block.inner(chunks[1]);
    frame.render_widget(query_block, chunks[1]);
    state.search_query.render_with_placeholder(
        frame,
        inner,
        Style::default().fg(theme.fg),
        Span::styled(t().type_search_text(), Style::default().fg(theme.muted)),
        state.search_focus == 0,
    );

    // Options row: limit (the rest of the retrieval is server-owned)
    let opts = Layout::horizontal([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(chunks[2]);
    let style = if state.search_focus == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let block = Block::default()
        .title(format!(" {} ", t().label_limit()))
        .borders(Borders::ALL)
        .border_style(style);
    let text = Paragraph::new(state.search_limit.to_string())
        .block(block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(text, opts[0]);

    // Results
    render_search_results(frame, chunks[3], state, theme);
}

fn render_search_results(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let title = t().rag_results_title(
        state.search_results.len(),
        state.is_loading,
        state.search_incomplete(),
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.search_results.is_empty() {
        let msg = if state.is_loading {
            t().searching()
        } else {
            t().no_rag_results()
        };
        let empty = Paragraph::new(msg).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .search_results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let style = if i == state.selected_result {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let content_preview = r
                .passages
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let title_str = r.fields.get("title").and_then(|v| v.as_str()).unwrap_or("");

            let preview = if !title_str.is_empty() {
                format!("[{}] {}", title_str, truncate_str(&content_preview, 60))
            } else {
                truncate_str(&content_preview, 80).to_string()
            };

            let text = format!("  {:.4}  {}", r.relevance, preview);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ==================== Import tab ====================

fn render_import_tab(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Title
        Constraint::Min(6),    // Content
        Constraint::Length(3), // Options row
        Constraint::Length(3), // Result
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_fields())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_import())),
        Span::styled("[Space]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_switch_mode())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Title input
    let title_style = if state.import_form_field == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let title_block = Block::default()
        .title(format!(" {} ", t().label_title_optional()))
        .borders(Borders::ALL)
        .border_style(title_style);
    let inner = title_block.inner(chunks[1]);
    frame.render_widget(title_block, chunks[1]);
    state.import_title.render_with_placeholder(
        frame,
        inner,
        Style::default().fg(theme.fg),
        Span::styled(t().title_placeholder(), Style::default().fg(theme.muted)),
        state.import_form_field == 1,
    );

    // Content input
    let content_style = if state.import_form_field == 0 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let content = state.import_content.text();
    let content_block = Block::default()
        .title(t().content_title(content.chars().count()))
        .borders(Borders::ALL)
        .border_style(content_style);
    let inner = content_block.inner(chunks[2]);
    frame.render_widget(content_block, chunks[2]);
    state.import_content.render(
        frame,
        inner,
        Style::default(),
        None,
        &plain(Style::default().fg(theme.fg)),
        state.import_form_field == 0,
    );
    if state.import_content.is_empty() {
        frame.render_widget(
            Paragraph::new(t().content_placeholder()).style(Style::default().fg(theme.muted)),
            inner,
        );
    }

    // Options row: chunk_size | overlap | mode
    let opts = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(30),
        Constraint::Percentage(35),
    ])
    .split(chunks[3]);

    let opt_items = [
        (
            2,
            t().label_chunk_size(),
            state.import_chunk_size.to_string(),
        ),
        (3, t().label_overlap(), state.import_overlap.to_string()),
        (4, t().label_mode(), state.import_mode.clone()),
    ];

    for (field_idx, label, value) in &opt_items {
        let style = if state.import_form_field == *field_idx {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.muted)
        };
        let idx = *field_idx - 2;
        let block = Block::default()
            .title(format!(" {} ", label))
            .borders(Borders::ALL)
            .border_style(style);
        let text = Paragraph::new(value.as_str())
            .block(block)
            .style(Style::default().fg(theme.fg));
        frame.render_widget(text, opts[idx]);
    }

    // Result
    if let Some(ref result) = state.import_result {
        let msg = import_result_text(result);
        let result_text = Paragraph::new(msg)
            .style(Style::default().fg(theme.success))
            .block(
                Block::default()
                    .title(format!(" {} ", t().label_result()))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.success)),
            );
        frame.render_widget(result_text, chunks[4]);
    } else {
        let hint = Paragraph::new(t().collection_label(&state.collection))
            .style(Style::default().fg(theme.muted))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.muted)),
            );
        frame.render_widget(hint, chunks[4]);
    }
}

// ==================== Collections tab ====================

fn render_collections_tab(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    if state.creating {
        render_create_collection_form(frame, area, state, theme);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(6),    // Collection list
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[c]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_create())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_stats())),
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().cmd_refresh())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Collection list
    let title = t().rag_collections_title(state.rag_collections.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.rag_collections.is_empty() {
        let empty =
            Paragraph::new(t().no_rag_collections()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .rag_collections
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let style = if i == state.selected_collection {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            ListItem::new(rag_collection_line(c)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_create_collection_form(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Name
        Constraint::Length(3), // Embedding field
        Constraint::Length(3), // Text field
        Constraint::Length(3), // Provider
        Constraint::Length(3), // Language
        Constraint::Min(1),    // Spacer
    ])
    .split(area);

    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_fields())),
        Span::styled("[Space]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_toggle())),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_create())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_cancel())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    let labels = [
        t().label_collection_name(),
        t().label_embedding_field(),
        t().label_text_field(),
        t().label_provider_space(),
        t().label_language_space(),
    ];
    for (field_idx, label) in labels.into_iter().enumerate() {
        let focused = state.create_form_field == field_idx;
        let block = Block::default()
            .title(format!(" {} ", label))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused { theme.accent } else { theme.muted }));
        let area = chunks[field_idx + 1];
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let fg = Style::default().fg(theme.fg);
        match field_idx {
            0 => state
                .create_collection_name
                .render(frame, inner, fg, focused),
            1 => state
                .create_embedding_field
                .render(frame, inner, fg, focused),
            2 => state.create_text_field.render(frame, inner, fg, focused),
            // Provider and language are toggled, not typed
            3 => frame.render_widget(
                Paragraph::new(state.create_provider.as_str()).style(fg),
                inner,
            ),
            _ => frame.render_widget(
                Paragraph::new(state.create_language.as_str()).style(fg),
                inner,
            ),
        }
    }
}

// ==================== Models tab ====================

fn render_models_tab(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    if state.models_mode == 1 {
        render_embed_test(frame, area, state, theme);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(6),    // Models list
        Constraint::Length(5), // Auto-embed & cache info
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[e]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_enable_auto_embed())),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_disable())),
        Span::styled("[c]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_cache_stats())),
        Span::styled("[x]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_clear_cache())),
        Span::styled("[t]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_test_embed())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Models list
    let title = t().embedding_models_title(state.models.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.models.is_empty() {
        let empty = Paragraph::new(t().loading_models()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
    } else {
        let items: Vec<ListItem> = state
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let style = if i == state.selected_model {
                    Style::default().fg(theme.bg).bg(theme.accent)
                } else {
                    Style::default().fg(theme.fg)
                };

                ListItem::new(model_line(m)).style(style)
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, inner);
    }

    // Auto-embed & cache info
    let info_chunks = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    // Auto-embed status
    let ae_block = Block::default()
        .title(format!(" {} ", t().auto_embed_title()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let ae_text = auto_embed_text(state.auto_embed_config.as_ref());
    let ae_para = Paragraph::new(ae_text)
        .block(ae_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(ae_para, info_chunks[0]);

    // Cache stats
    let cache_block = Block::default()
        .title(format!(" {} ", t().cache_title()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let cache_text = cache_text(state.cache_stats.as_ref());
    let cache_para = Paragraph::new(cache_text)
        .block(cache_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(cache_para, info_chunks[1]);
}

fn render_embed_test(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Length(3), // Input
        Constraint::Min(4),    // Result
    ])
    .split(area);

    let help = Paragraph::new(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_embed())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_back())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    let input_block = Block::default()
        .title(format!(" {} ", t().text_to_embed_title()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));
    let inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);
    state.embed_test_input.render_with_placeholder(
        frame,
        inner,
        Style::default().fg(theme.fg),
        Span::styled(t().type_text_here(), Style::default().fg(theme.muted)),
        true,
    );

    let result_block = Block::default()
        .title(format!(" {} ", t().label_result()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let result_text = match state.embed_test_result {
        Some(ref result) => embed_result_text(result),
        None => t().no_result_yet().to_string(),
    };
    let result_para = Paragraph::new(result_text)
        .block(result_block)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .style(Style::default().fg(theme.fg));
    frame.render_widget(result_para, chunks[2]);
}

// ==================== Jobs tab ====================

fn render_jobs_tab(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let chunks = Layout::vertical([
        Constraint::Length(2), // Help
        Constraint::Min(6),    // Job list
    ])
    .split(area);

    // Help text
    let help = Paragraph::new(Line::from(vec![
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_navigate())),
        Span::styled("[x]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().key_cancel_job())),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}  ", t().cmd_refresh())),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(format!(" {}", t().key_close())),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Job list
    let title = t().jobs_title(state.jobs.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.jobs.is_empty() {
        let empty = Paragraph::new(t().no_jobs()).style(Style::default().fg(theme.muted));
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = state
        .jobs
        .iter()
        .enumerate()
        .map(|(i, j)| {
            let style = if i == state.selected_job {
                Style::default().fg(theme.bg).bg(theme.accent)
            } else {
                Style::default().fg(theme.fg)
            };

            let text = job_line(j);
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ==================== Status bar ====================

fn render_status(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let status_text = if let Some(ref err) = state.error {
        Line::from(vec![
            Span::styled(
                format!("{}: ", t().label_error()),
                Style::default().fg(theme.error),
            ),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            msg.clone(),
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            t().collection_label(&state.collection),
            Style::default().fg(theme.muted),
        )])
    };

    let status = Paragraph::new(status_text);
    frame.render_widget(status, area);
}

// ==================== Helpers ====================

/// The outcome of a document import: chunks stored, or why none were
pub(crate) fn import_result_text(result: &RagDocumentImportResponse) -> String {
    match result {
        RagDocumentImportResponse::Imported(r) => t().import_result(
            &r.doc_id,
            r.chunks_created,
            r.chunks_replaced,
            r.language_ignored,
        ),
        RagDocumentImportResponse::Skipped(r) => format!("„{}”: {}", r.doc_id, r.message),
        RagDocumentImportResponse::Empty(r) => r.message.clone(),
    }
}

/// A RAG collection of the Collections tab: chunks, source documents and
/// provider (read from the nested `stats` and `config`)
pub(crate) fn rag_collection_line(c: &RagCollectionStatsResponse) -> String {
    let provider = c.config.as_ref().map(|cfg| cfg.provider.as_str());
    format!(
        "  {}",
        t().rag_collection_row(
            &c.collection,
            &c.stats.chunk_count,
            &c.stats.source_document_count,
            provider
        )
    )
}

/// A provider of the Models tab: name, model and dimension
fn model_line(m: &ModelInfo) -> String {
    format!(
        "  {}",
        t().model_row(&m.provider, &m.model, m.dimension, m.available)
    )
}

/// The collection's auto-embedding: source → target field and provider, and
/// what still needs attention (a pending re-embed, a removed provider)
fn auto_embed_text(status: Option<&AutoEmbedStatusResponse>) -> String {
    match status {
        None => t().not_loaded().to_string(),
        Some(AutoEmbedStatusResponse::NotConfigured { .. }) => t().auto_embed_off().to_string(),
        Some(AutoEmbedStatusResponse::Configured {
            enabled,
            config,
            reembed_pending,
            requires_reconfiguration,
            ..
        }) => t().auto_embed_status(
            *enabled,
            &config.source_field,
            &config.target_field,
            &config.provider,
            *reembed_pending,
            requires_reconfiguration.unwrap_or(false),
        ),
    }
}

/// The embedding cache: entries and hit rate (already a percentage)
fn cache_text(stats: Option<&EmbedCacheStatsResponse>) -> String {
    match stats {
        None => t().cache_not_loaded().to_string(),
        Some(EmbedCacheStatsResponse::Disabled { .. }) => {
            t().cache_disabled_on_server().to_string()
        }
        Some(EmbedCacheStatsResponse::Enabled { stats, .. }) => t().cache_stats(
            stats.entries,
            stats.max_entries,
            stats.hit_rate,
            stats.hits,
            stats.misses,
        ),
    }
}

/// A test embedding: provider, model, dimension and the first values
fn embed_result_text(result: &EmbedTextResponse) -> String {
    let preview: Vec<String> = result
        .vector
        .iter()
        .take(5)
        .map(|v| format!("{:.4}", v))
        .collect();
    let more = if result.vector.len() > 5 { ", ..." } else { "" };
    t().embed_result(
        &result.provider,
        &result.model,
        result.dimension,
        &format!("{}{}", preview.join(", "), more),
    )
}

/// A job of the Jobs tab: id, state, what it works on and its progress
fn job_line(job: &JobInfo) -> String {
    let status = match &job.status {
        JobStatus::Pending => t().job_pending().to_string(),
        JobStatus::Running { progress, .. } => t().job_running(*progress),
        JobStatus::Completed { .. } => t().job_completed().to_string(),
        JobStatus::Failed { error } => t().job_failed(error),
        JobStatus::Cancelled => t().job_cancelled().to_string(),
    };
    let target = match &job.job_type {
        JobType::EmbedBackfill { collection, .. } => t().job_backfill(collection),
        JobType::EmbedDocument { collection, doc_id } => t().job_import(collection, doc_id),
        JobType::Compact => t().job_compact().to_string(),
        JobType::Custom { name } => name.clone(),
    };
    let progress = match job.items_total {
        Some(total) if total > 0 => format!(" {}/{}", job.items_processed, total),
        _ => String::new(),
    };
    format!(
        "  {} | {} | {}{}",
        truncate_str(&job.id, 12),
        status,
        target,
        progress
    )
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironbase_mcp_types::responses::{AutoEmbedConfigView, CacheStats, Flag};

    fn configured(reembed_pending: bool, removed: bool) -> AutoEmbedStatusResponse {
        AutoEmbedStatusResponse::Configured {
            collection: "kb".to_string(),
            enabled: true,
            config: Box::new(AutoEmbedConfigView {
                source_field: "content".to_string(),
                target_field: "embedding".to_string(),
                provider: "vllm".to_string(),
                model: Some("bge-m3".to_string()),
                dimension: Some(1024),
                skip_if_exists: false,
                chunking: None,
            }),
            reembed_pending,
            reembed_field: None,
            requires_reconfiguration: removed.then_some(true),
            reconfiguration_reason: None,
        }
    }

    /// The fields are read from the nested `config` (before: from the top
    /// level, where they are not: "Enabled: - -> -")
    #[test]
    fn auto_embed_text_reads_the_nested_config() {
        assert_eq!(
            auto_embed_text(Some(&configured(false, false))),
            "Bekapcsolva: content -> embedding (vllm)"
        );
        assert_eq!(
            auto_embed_text(Some(&configured(true, true))),
            "Bekapcsolva: content -> embedding (vllm) | újrabeágyazás folyamatban | a szolgáltató megszűnt: kapcsold be újra"
        );
        let none = AutoEmbedStatusResponse::NotConfigured {
            collection: "kb".to_string(),
            enabled: Flag,
            config: (),
        };
        assert_eq!(auto_embed_text(Some(&none)), "Kikapcsolva");
        assert_eq!(auto_embed_text(None), "Nincs betöltve");
    }

    /// The stats are read from the nested `stats`, and the hit rate is
    /// already a percentage (before: "0 entries | 0.0% hit rate" always, and
    /// a real rate would have been multiplied by 100 again)
    #[test]
    fn cache_text_reads_the_nested_stats_as_a_percentage() {
        let stats = EmbedCacheStatsResponse::Enabled {
            enabled: Flag,
            stats: CacheStats {
                hits: 1,
                misses: 1,
                entries: 1,
                max_entries: 100,
                bytes_used: 84,
                hit_rate: 50.0,
            },
        };
        assert_eq!(
            cache_text(Some(&stats)),
            "1/100 bejegyzés | 50.0% találati arány (1 találat, 1 hiány)"
        );
        let disabled = EmbedCacheStatsResponse::Disabled {
            enabled: Flag,
            message: "Embedding cache is not enabled".to_string(),
        };
        assert_eq!(cache_text(Some(&disabled)), "A szerveren kikapcsolva");
    }

    /// The vector is read from `vector` (before: from `embedding`: "N/A")
    #[test]
    fn embed_result_text_previews_the_vector() {
        let result = EmbedTextResponse {
            vector: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
            dimension: 6,
            provider: "vllm".to_string(),
            model: "bge-m3".to_string(),
        };
        assert_eq!(
            embed_result_text(&result),
            "Szolgáltató: vllm (bge-m3) | Dim: 6 | [0.1000, 0.2000, 0.3000, 0.4000, 0.5000, ...]"
        );
    }
}
