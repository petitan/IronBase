//! Main UI renderer

use crate::app::{App, ConnectionType, Modal, Pane};
use crate::modals;
use crate::panes;
use crate::theme::Theme;
use ratatui::prelude::*;

/// Render the entire UI
pub fn render_ui(frame: &mut Frame, app: &App) {
    let theme = app.theme.clone();

    // Main layout: Header + Content + Command Bar
    let layout = Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(10),   // Content (3 panes)
        Constraint::Length(1), // Command bar
    ])
    .split(frame.area());

    // Background
    frame.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme.bg)),
        frame.area(),
    );

    // Header
    render_header(frame, layout[0], app, &theme);

    // 3-pane layout
    let panes_layout = Layout::horizontal([
        Constraint::Percentage(25), // Collections
        Constraint::Percentage(35), // Documents
        Constraint::Percentage(40), // Detail
    ])
    .split(layout[1]);

    panes::collections::render(
        frame,
        panes_layout[0],
        app,
        &theme,
        app.active_pane == Pane::Collections,
    );
    panes::documents::render(
        frame,
        panes_layout[1],
        app,
        &theme,
        app.active_pane == Pane::Documents,
    );
    panes::detail::render(
        frame,
        panes_layout[2],
        app,
        &theme,
        app.active_pane == Pane::Detail,
    );

    // Command bar
    render_command_bar(frame, layout[2], app, &theme);

    // Render modal if active
    if let Some(modal) = app.modal {
        match modal {
            Modal::Search => modals::search::render(frame, frame.area(), app, &theme),
            Modal::Actions => modals::actions::render(frame, frame.area(), app, &theme),
            Modal::Help => modals::help::render(frame, frame.area(), app.help_scroll, &theme),
            Modal::Confirm => modals::confirm::render(frame, frame.area(), &app.confirm, &theme),
            Modal::Insert => modals::insert::render(frame, frame.area(), &app.insert, &theme),
            Modal::Index => modals::index::render(frame, frame.area(), &app.index_state, &theme),
            Modal::Query => modals::query::render(frame, frame.area(), &app.query_state, &theme),
            Modal::Export => modals::export::render(frame, frame.area(), &app.export_state, &theme),
            Modal::Filter => modals::filter::render(frame, frame.area(), &app.filter_state, &theme),
            Modal::NewCollection => modals::new_collection::render(
                frame,
                frame.area(),
                &app.new_collection_state,
                &theme,
            ),
            Modal::ErrorDetail => {
                if let Some(ref err) = app.error_message {
                    modals::error::render(frame, frame.area(), err, app.error_scroll, &theme);
                }
            }
            Modal::Script => {
                modals::script::render(frame, frame.area(), &app.script_state, &theme);
            }
            Modal::ServerInfo => {
                modals::server_info::render(frame, frame.area(), &app.server_info_state, app.server_info_scroll, &theme);
            }
            Modal::Update => {
                modals::update::render(frame, frame.area(), &app.update_state, &theme);
            }
            Modal::Database => {
                modals::database::render(frame, frame.area(), &app.database_state, &theme);
            }
            Modal::ApiKey => {
                modals::api_key::render(frame, frame.area(), &app.api_key_state, &theme);
            }
            Modal::Fulltext => {
                modals::fulltext::render(frame, frame.area(), &app.fulltext_state, &theme);
            }
            Modal::Acl => {
                modals::acl::render(frame, frame.area(), &app.acl_state, &theme);
            }
            Modal::Listener => {
                modals::listener::render(frame, frame.area(), &app.listener_state, &theme);
            }
            Modal::VectorSearch => {
                modals::vector::render(frame, frame.area(), &app.vector_state, &theme);
            }
            Modal::Rag => {
                modals::rag::render(frame, frame.area(), &app.rag_state, &theme);
            }
        }
    }
}

/// Render the header bar
pub fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let header_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // DB info with version
    let db_info = format!(
        " IronBase v{} | {} ({} coll, {} docs) ",
        env!("CARGO_PKG_VERSION"),
        app.db_name(),
        app.collections.len(),
        app.total_doc_count()
    );

    // Pane indicator
    let pane_name = match app.active_pane {
        Pane::Collections => "Collections",
        Pane::Documents => "Documents",
        Pane::Detail => "Detail",
    };

    // Connection type indicator
    let (conn_text, conn_color) = match app.connection_type {
        ConnectionType::Localhost => ("LOCAL", theme.success),
        ConnectionType::Internal => ("LAN", theme.warning),
        ConnectionType::External => ("EXT", theme.error),
        ConnectionType::Unknown => ("?", theme.muted),
    };

    // Build spans
    let spans = vec![
        Span::styled(db_info, Style::default().fg(theme.accent).bold()),
        Span::raw(" | "),
        Span::styled(pane_name, Style::default().fg(theme.fg)),
        Span::raw(" | "),
        Span::styled(format!("[{}]", conn_text), Style::default().fg(conn_color).bold()),
    ];

    let line = Line::from(spans).patch_style(header_style);

    frame.render_widget(Paragraph::new(line), area);
}

/// Render the command bar at the bottom
pub fn render_command_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let bar_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // Context-dependent commands based on active pane
    let commands: Vec<(&str, String)> = match app.active_pane {
        Pane::Collections => vec![
            ("Tab", "Panel".into()),
            ("j/k", "Navigate".into()),
            ("Enter", "Select".into()),
            ("r", "Refresh".into()),
            ("f", "Filter".into()),
            ("/", "Search".into()),
            ("a", "Actions".into()),
            ("?", "Sugo".into()),
            ("q", "Quit".into()),
        ],
        Pane::Documents => vec![
            ("Tab", "Panel".into()),
            ("j/k", "Navigate".into()),
            ("PgUp/Dn", "Page".into()),
            ("r", "Refresh".into()),
            ("f", "Filter".into()),
            ("/", "Search".into()),
            ("a", "Actions".into()),
            ("?", "Sugo".into()),
            ("q", "Quit".into()),
        ],
        Pane::Detail => {
            let mut cmds = vec![
                ("Tab", "Panel".into()),
                ("j/k", "Scroll".into()),
                ("/", "Search".into()),
            ];
            // Only show n/N if there are search matches
            if !app.search.doc_matches.is_empty() {
                let current = app.search.current_match + 1;
                let total = app.search.doc_matches.len();
                cmds.push(("n/N", format!("{}/{}", current, total)));
            }
            cmds.extend([
                ("e", "Edit".into()),
                ("d", "Delete".into()),
                ("y", "Copy".into()),
                ("?", "Sugo".into()),
                ("q", "Quit".into()),
            ]);
            cmds
        }
    };

    let mut spans = Vec::new();
    spans.push(Span::raw(" "));

    for (key, action) in commands {
        spans.push(Span::styled(
            format!("[{}]", key),
            Style::default().fg(theme.accent),
        ));
        spans.push(Span::raw(format!(" {} ", action)));
    }

    // Add error/status message if present
    if let Some(ref err) = app.error_message {
        // Truncate long errors for status bar display (UTF-8 safe)
        let max_err_len = 50;
        let truncated = if err.chars().count() > max_err_len {
            let chars: String = err.chars().take(max_err_len).collect();
            format!("{}...", chars)
        } else {
            err.clone()
        };
        spans.push(Span::styled(
            format!(" | Hiba: {}", truncated),
            Style::default().fg(theme.error),
        ));
        // Add hint to open error detail modal
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[e]", Style::default().fg(theme.accent)));
        spans.push(Span::styled(" Reszletek", Style::default().fg(theme.muted)));
    } else if let Some(ref msg) = app.status_message {
        spans.push(Span::styled(
            format!(" | {}", msg),
            Style::default().fg(theme.muted),
        ));
    }

    let line = Line::from(spans).patch_style(bar_style);
    frame.render_widget(Paragraph::new(line), area);
}
