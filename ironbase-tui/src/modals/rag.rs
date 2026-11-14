//! RAG & Embedding modal - search, import, collections, models, jobs

use super::render_modal_frame;
use crate::state::rag::{RagState, RagTab};
use crate::theme::Theme;
use ironbase_mcp_types::responses::{
    AutoEmbedStatusResponse, EmbedCacheStatsResponse, EmbedTextResponse, JobInfo, JobStatus,
    JobType, ModelInfo, RagCollectionStatsResponse, RagDocumentImportResponse,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};

/// Render the RAG & Embedding modal
pub fn render(frame: &mut Frame, area: Rect, state: &RagState, theme: &Theme) {
    let inner = render_modal_frame(frame, area, "RAG & Embedding", theme, 85, 90);

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
    let titles = vec!["Search", "Import", "Collections", "Models", "Jobs"];
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
        Span::raw(" Tabs  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Search  "),
        Span::styled("[↑/↓]", Style::default().fg(theme.accent)),
        Span::raw(" Navigate  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
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
        .title(" Query ")
        .borders(Borders::ALL)
        .border_style(query_style);
    let query_preview = if state.search_query.is_empty() {
        "Enter search query...".to_string()
    } else {
        format!("{}|", state.search_query)
    };
    let query_text =
        Paragraph::new(query_preview)
            .block(query_block)
            .style(if state.search_query.is_empty() {
                Style::default().fg(theme.muted)
            } else {
                Style::default().fg(theme.fg)
            });
    frame.render_widget(query_text, chunks[1]);

    // Options row: limit (the rest of the retrieval is server-owned)
    let opts = Layout::horizontal([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(chunks[2]);
    let style = if state.search_focus == 1 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let block = Block::default()
        .title(" Limit ")
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
    let title = if state.is_loading {
        " Results (loading...) ".to_string()
    } else if state.search_incomplete() {
        format!(" Results ({}, trimmed) ", state.search_results.len())
    } else {
        format!(" Results ({}) ", state.search_results.len())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.search_results.is_empty() {
        let msg = if state.is_loading {
            "Searching..."
        } else {
            "No results. Enter query and press Enter."
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
        Span::styled("[Ctrl+Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Fields  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Import  "),
        Span::styled("[Space]", Style::default().fg(theme.accent)),
        Span::raw(" Toggle mode  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
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
        .title(" Title (optional) ")
        .borders(Borders::ALL)
        .border_style(title_style);
    let title_text = Paragraph::new(if state.import_title.is_empty() {
        "Document title...".to_string()
    } else {
        state.import_title.clone()
    })
    .block(title_block)
    .style(if state.import_title.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    });
    frame.render_widget(title_text, chunks[1]);

    // Content input
    let content_style = if state.import_form_field == 0 {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.muted)
    };
    let content_block = Block::default()
        .title(format!(
            " Content ({} chars) ",
            state.import_content.chars().count()
        ))
        .borders(Borders::ALL)
        .border_style(content_style);
    let content_preview = if state.import_content.is_empty() {
        "Paste or type content here...".to_string()
    } else {
        truncate_str(&state.import_content, 500).to_string()
    };
    let content_text = Paragraph::new(content_preview)
        .block(content_block)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .style(if state.import_content.is_empty() {
            Style::default().fg(theme.muted)
        } else {
            Style::default().fg(theme.fg)
        });
    frame.render_widget(content_text, chunks[2]);

    // Options row: chunk_size | overlap | mode
    let opts = Layout::horizontal([
        Constraint::Percentage(35),
        Constraint::Percentage(30),
        Constraint::Percentage(35),
    ])
    .split(chunks[3]);

    let opt_items = [
        (2, "Chunk Size", state.import_chunk_size.to_string()),
        (3, "Overlap", state.import_overlap.to_string()),
        (4, "Mode", state.import_mode.clone()),
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
                    .title(" Result ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.success)),
            );
        frame.render_widget(result_text, chunks[4]);
    } else {
        let hint = Paragraph::new(format!("Collection: {}", state.collection))
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
        Span::raw(" Create  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Stats  "),
        Span::styled("[j/k]", Style::default().fg(theme.accent)),
        Span::raw(" Navigate  "),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(" Refresh  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Collection list
    let title = format!(" RAG Collections ({}) ", state.rag_collections.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.rag_collections.is_empty() {
        let empty = Paragraph::new("No RAG collections. Press [c] to create one.")
            .style(Style::default().fg(theme.muted));
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
        Span::styled("[Ctrl+Tab]", Style::default().fg(theme.accent)),
        Span::raw(" Fields  "),
        Span::styled("[Space]", Style::default().fg(theme.accent)),
        Span::raw(" Toggle  "),
        Span::styled("[Enter]", Style::default().fg(theme.accent)),
        Span::raw(" Create  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Cancel"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    let fields = [
        (0, "Collection Name", &state.create_collection_name),
        (1, "Embedding Field", &state.create_embedding_field),
        (2, "Text Field", &state.create_text_field),
        (3, "Provider [Space]", &state.create_provider),
        (4, "Language [Space]", &state.create_language),
    ];

    for (field_idx, label, value) in &fields {
        let style = if state.create_form_field == *field_idx {
            Style::default().fg(theme.accent)
        } else {
            Style::default().fg(theme.muted)
        };
        let cursor = if state.create_form_field == *field_idx {
            "|"
        } else {
            ""
        };
        let block = Block::default()
            .title(format!(" {} ", label))
            .borders(Borders::ALL)
            .border_style(style);
        let text = Paragraph::new(format!("{}{}", value, cursor))
            .block(block)
            .style(Style::default().fg(theme.fg));
        frame.render_widget(text, chunks[*field_idx + 1]);
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
        Span::raw(" Enable auto-embed  "),
        Span::styled("[d]", Style::default().fg(theme.accent)),
        Span::raw(" Disable  "),
        Span::styled("[c]", Style::default().fg(theme.accent)),
        Span::raw(" Cache stats  "),
        Span::styled("[x]", Style::default().fg(theme.accent)),
        Span::raw(" Clear cache  "),
        Span::styled("[t]", Style::default().fg(theme.accent)),
        Span::raw(" Test embed"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Models list
    let title = format!(" Embedding Models ({}) ", state.models.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.models.is_empty() {
        let empty = Paragraph::new("Loading models...").style(Style::default().fg(theme.muted));
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
        .title(" Auto-Embed ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let ae_text = auto_embed_text(state.auto_embed_config.as_ref());
    let ae_para = Paragraph::new(ae_text)
        .block(ae_block)
        .style(Style::default().fg(theme.fg));
    frame.render_widget(ae_para, info_chunks[0]);

    // Cache stats
    let cache_block = Block::default()
        .title(" Cache ")
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
        Span::raw(" Embed  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Back"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    let input_block = Block::default()
        .title(" Text to embed ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));
    let input_text = Paragraph::new(if state.embed_test_input.is_empty() {
        "Type text here...".to_string()
    } else {
        format!("{}|", state.embed_test_input)
    })
    .block(input_block)
    .style(if state.embed_test_input.is_empty() {
        Style::default().fg(theme.muted)
    } else {
        Style::default().fg(theme.fg)
    });
    frame.render_widget(input_text, chunks[1]);

    let result_block = Block::default()
        .title(" Result ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.muted));
    let result_text = match state.embed_test_result {
        Some(ref result) => embed_result_text(result),
        None => "No result yet".to_string(),
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
        Span::raw(" Navigate  "),
        Span::styled("[x]", Style::default().fg(theme.accent)),
        Span::raw(" Cancel job  "),
        Span::styled("[r]", Style::default().fg(theme.accent)),
        Span::raw(" Refresh  "),
        Span::styled("[Esc]", Style::default().fg(theme.accent)),
        Span::raw(" Close"),
    ]))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(help, chunks[0]);

    // Job list
    let title = format!(" Jobs ({}) ", state.jobs.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    if state.jobs.is_empty() {
        let empty = Paragraph::new("No jobs. Press [r] to refresh.")
            .style(Style::default().fg(theme.muted));
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
            Span::styled("Error: ", Style::default().fg(theme.error)),
            Span::styled(err.clone(), Style::default().fg(theme.error)),
        ])
    } else if let Some(ref msg) = state.message {
        Line::from(vec![Span::styled(
            msg.clone(),
            Style::default().fg(theme.accent),
        )])
    } else {
        Line::from(vec![Span::styled(
            format!("Collection: {}", state.collection),
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
        RagDocumentImportResponse::Imported(r) => {
            let mut text = format!(
                "Imported '{}': {} chunks created",
                r.doc_id, r.chunks_created
            );
            if r.chunks_replaced > 0 {
                text.push_str(&format!(", {} replaced", r.chunks_replaced));
            }
            if r.language_ignored {
                text.push_str(" (language ignored: the collection keeps its own)");
            }
            text
        }
        RagDocumentImportResponse::Skipped(r) => format!("'{}': {}", r.doc_id, r.message),
        RagDocumentImportResponse::Empty(r) => r.message.clone(),
    }
}

/// A RAG collection of the Collections tab: chunks, source documents and
/// provider (read from the nested `stats` and `config`)
pub(crate) fn rag_collection_line(c: &RagCollectionStatsResponse) -> String {
    let provider = c
        .config
        .as_ref()
        .map_or("no RAG config", |cfg| &cfg.provider);
    format!(
        "  {} | {} chunks | {} docs | {}",
        c.collection, c.stats.chunk_count, c.stats.source_document_count, provider
    )
}

/// A provider of the Models tab: name, model and dimension
fn model_line(m: &ModelInfo) -> String {
    let status = if m.available { "OK" } else { "N/A" };
    format!(
        "  {} | {} | dim:{} | {}",
        m.provider, m.model, m.dimension, status
    )
}

/// The collection's auto-embedding: source → target field and provider, and
/// what still needs attention (a pending re-embed, a removed provider)
fn auto_embed_text(status: Option<&AutoEmbedStatusResponse>) -> String {
    match status {
        None => "Not loaded".to_string(),
        Some(AutoEmbedStatusResponse::NotConfigured { .. }) => "Disabled".to_string(),
        Some(AutoEmbedStatusResponse::Configured {
            enabled,
            config,
            reembed_pending,
            requires_reconfiguration,
            ..
        }) => {
            let mut text = format!(
                "{}: {} -> {} ({})",
                if *enabled {
                    "Enabled"
                } else {
                    "Configured, off"
                },
                config.source_field,
                config.target_field,
                config.provider
            );
            if *reembed_pending {
                text.push_str(" | re-embed pending");
            }
            if requires_reconfiguration.unwrap_or(false) {
                text.push_str(" | provider removed: enable again");
            }
            text
        }
    }
}

/// The embedding cache: entries and hit rate (already a percentage)
fn cache_text(stats: Option<&EmbedCacheStatsResponse>) -> String {
    match stats {
        None => "Press [c] to load".to_string(),
        Some(EmbedCacheStatsResponse::Disabled { .. }) => "Disabled on the server".to_string(),
        Some(EmbedCacheStatsResponse::Enabled { stats, .. }) => format!(
            "{}/{} entries | {:.1}% hit rate ({} hits, {} misses)",
            stats.entries, stats.max_entries, stats.hit_rate, stats.hits, stats.misses
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
    format!(
        "Provider: {} ({}) | Dim: {} | [{}{}]",
        result.provider,
        result.model,
        result.dimension,
        preview.join(", "),
        more
    )
}

/// A job of the Jobs tab: id, state, what it works on and its progress
fn job_line(job: &JobInfo) -> String {
    let status = match &job.status {
        JobStatus::Pending => "pending".to_string(),
        JobStatus::Running { progress, .. } => format!("running {:.0}%", progress),
        JobStatus::Completed { .. } => "completed".to_string(),
        JobStatus::Failed { error } => format!("failed: {}", error),
        JobStatus::Cancelled => "cancelled".to_string(),
    };
    let target = match &job.job_type {
        JobType::EmbedBackfill { collection, .. } => format!("backfill {}", collection),
        JobType::EmbedDocument { collection, doc_id } => {
            format!("import {} / {}", collection, doc_id)
        }
        JobType::Compact => "compact".to_string(),
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
            "Enabled: content -> embedding (vllm)"
        );
        assert_eq!(
            auto_embed_text(Some(&configured(true, true))),
            "Enabled: content -> embedding (vllm) | re-embed pending | provider removed: enable again"
        );
        let none = AutoEmbedStatusResponse::NotConfigured {
            collection: "kb".to_string(),
            enabled: Flag,
            config: (),
        };
        assert_eq!(auto_embed_text(Some(&none)), "Disabled");
        assert_eq!(auto_embed_text(None), "Not loaded");
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
            "1/100 entries | 50.0% hit rate (1 hits, 1 misses)"
        );
        let disabled = EmbedCacheStatsResponse::Disabled {
            enabled: Flag,
            message: "Embedding cache is not enabled".to_string(),
        };
        assert_eq!(cache_text(Some(&disabled)), "Disabled on the server");
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
            "Provider: vllm (bge-m3) | Dim: 6 | [0.1000, 0.2000, 0.3000, 0.4000, 0.5000, ...]"
        );
    }
}
