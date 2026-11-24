//! Main UI renderer

use crate::app::{App, ConnectionType, Modal, OpenModal, Pane};
use crate::i18n::t;
use crate::modals;
use crate::panes;
use crate::theme::Theme;
use ratatui::prelude::*;

/// Main layout: Header + Content + Command Bar
fn main_layout(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(1), // Header
        Constraint::Min(10),   // Content (3 panes)
        Constraint::Length(1), // Command bar
    ])
    .split(area)
}

/// Height of the content area (the 3 panes) for a given terminal height
pub fn content_height(terminal_height: u16) -> u16 {
    main_layout(Rect::new(0, 0, 1, terminal_height))[1].height
}

/// Render the entire UI
pub fn render_ui(frame: &mut Frame, app: &App) {
    let theme = app.theme.clone();

    let layout = main_layout(frame.area());

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
    if let Some(modal) = app.modal() {
        match modal {
            Modal::Search => modals::search::render(frame, frame.area(), app, &theme),
            Modal::Actions => modals::actions::render(frame, frame.area(), app, &theme),
            Modal::Help => modals::help::render(frame, frame.area(), app.help_scroll, &theme),
            Modal::Confirm => {
                if let Some(state) = app.confirm_state() {
                    modals::confirm::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Insert => {
                if let Some(state) = app.insert_state() {
                    modals::insert::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Index => {
                if let Some(state) = app.index_state() {
                    modals::index::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Query => {
                if let Some(state) = app.query_state() {
                    modals::query::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Export => {
                if let Some(state) = app.export_state() {
                    modals::export::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Filter => modals::filter::render(frame, frame.area(), &app.filter_state, &theme),
            Modal::NewCollection => {
                if let Some(OpenModal::NewCollection(state)) = app.open_modal() {
                    modals::new_collection::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::ErrorDetail => {
                if let Some(ref err) = app.error_message {
                    modals::error::render(frame, frame.area(), err, app.error_scroll, &theme);
                }
            }
            Modal::Script => {
                if let Some(state) = app.script_state() {
                    modals::script::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::ServerInfo => {
                if let Some(OpenModal::ServerInfo(state)) = app.open_modal() {
                    modals::server_info::render(
                        frame,
                        frame.area(),
                        state,
                        app.server_info_scroll,
                        &theme,
                    );
                }
            }
            Modal::Update => {
                if let Some(OpenModal::Update(state)) = app.open_modal() {
                    modals::update::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Database => {
                if let Some(state) = app.database_state() {
                    modals::database::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::ApiKey => {
                if let Some(state) = app.api_key_state() {
                    modals::api_key::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Fulltext => {
                if let Some(state) = app.fulltext_state() {
                    modals::fulltext::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Acl => {
                if let Some(state) = app.acl_state() {
                    modals::acl::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Listener => {
                if let Some(state) = app.listener_state() {
                    modals::listener::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::VectorSearch => {
                if let Some(state) = app.vector_state() {
                    modals::vector::render(frame, frame.area(), state, &theme);
                }
            }
            Modal::Rag => {
                if let Some(state) = app.rag_state() {
                    modals::rag::render(frame, frame.area(), state, &theme);
                }
            }
        }
    }
}

/// Render the header bar
pub fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let header_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // DB info with version
    let db_info = t().header_db(
        env!("CARGO_PKG_VERSION"),
        app.db_name(),
        app.collections.len(),
        app.total_doc_count(),
    );

    // Pane indicator
    let pane_name = match app.active_pane {
        Pane::Collections => t().pane_collections(),
        Pane::Documents => t().pane_documents(),
        Pane::Detail => t().pane_detail(),
    };

    // Connection type indicator
    let (conn_text, conn_color) = match app.connection_type {
        ConnectionType::Localhost => ("LOCAL", theme.success),
        ConnectionType::Internal => ("LAN", theme.warning),
        ConnectionType::External => ("EXT", theme.error),
        ConnectionType::Unknown => ("?", theme.muted),
    };

    // The connection first: on a narrow terminal the end of the line is cut,
    // and `[EXT]` is a warning that must stay visible
    let spans = vec![
        Span::styled(
            format!(" [{}]", conn_text),
            Style::default().fg(conn_color).bold(),
        ),
        Span::styled(db_info, Style::default().fg(theme.accent).bold()),
        Span::raw("| "),
        Span::styled(pane_name, Style::default().fg(theme.fg)),
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
            ("Tab", t().cmd_panel().into()),
            ("j/k", t().cmd_navigate().into()),
            ("Enter", t().cmd_select().into()),
            ("r", t().cmd_refresh().into()),
            ("f", t().cmd_filter().into()),
            ("/", t().cmd_search().into()),
            ("a", t().cmd_actions().into()),
            ("?", t().cmd_help().into()),
            ("q", t().cmd_quit().into()),
        ],
        Pane::Documents => vec![
            ("Tab", t().cmd_panel().into()),
            ("j/k", t().cmd_navigate().into()),
            ("PgUp/Dn", t().cmd_page().into()),
            ("r", t().cmd_refresh().into()),
            ("f", t().cmd_filter().into()),
            ("/", t().cmd_search().into()),
            ("a", t().cmd_actions().into()),
            ("?", t().cmd_help().into()),
            ("q", t().cmd_quit().into()),
        ],
        Pane::Detail => {
            let mut cmds = vec![
                ("Tab", t().cmd_panel().into()),
                ("j/k", t().cmd_scroll().into()),
                ("/", t().cmd_search().into()),
            ];
            // Only show n/N if there are search matches
            if !app.search.doc_matches.is_empty() {
                let current = app.search.current_match + 1;
                let total = app.search.doc_matches.len();
                cmds.push(("n/N", format!("{}/{}", current, total)));
            }
            cmds.extend([
                ("e", t().cmd_edit().into()),
                ("d", t().cmd_delete().into()),
                ("y", t().cmd_copy().into()),
                ("?", t().cmd_help().into()),
                ("q", t().cmd_quit().into()),
            ]);
            cmds
        }
    };

    // The error / status first: on a narrow terminal the end of the bar is
    // cut, and the message matters more than the key hints
    let mut spans = Vec::new();
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
            t().bar_error(&truncated),
            Style::default().fg(theme.error),
        ));
        // Add hint to open error detail modal
        spans.push(Span::styled(" [e]", Style::default().fg(theme.accent)));
        spans.push(Span::styled(
            t().bar_error_details(),
            Style::default().fg(theme.muted),
        ));
        spans.push(Span::raw(" |"));
    } else if app.tasks.is_busy() {
        spans.push(Span::styled(
            format!(" {} |", t().loading_in_background()),
            Style::default().fg(theme.warning),
        ));
    } else if let Some(ref msg) = app.status_message {
        spans.push(Span::styled(
            format!(" {} |", msg),
            Style::default().fg(theme.muted),
        ));
    }
    spans.push(Span::raw(" "));

    for (key, action) in commands {
        spans.push(Span::styled(
            format!("[{}]", key),
            Style::default().fg(theme.accent),
        ));
        spans.push(Span::raw(format!(" {} ", action)));
    }

    let line = Line::from(spans).patch_style(bar_style);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::CollectionInfo;
    use ratatui::{backend::TestBackend, Terminal};

    /// The connection warning is the first thing in the header: a long
    /// database name or a longer translation cuts the end of the line on an
    /// 80-column terminal, not `[EXT]`
    #[test]
    fn test_header_keeps_the_connection_type_visible() {
        let mut app = App::new(Config::default());
        app.connection_type = ConnectionType::External;
        app.db_path = Some(std::path::PathBuf::from(
            "/data/a_rather_long_database_name.mlite",
        ));
        app.collections = (0..5)
            .map(|i| CollectionInfo::unloaded(format!("coll{}", i)))
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render_ui(f, &app)).unwrap();
        let header: String = (0..80)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert!(header.contains("[EXT]"), "{header}");
    }

    /// Rendering must survive any terminal size: `visible_height - 1` in the
    /// collections pane underflowed when the pane had no inner rows
    #[test]
    fn test_render_survives_tiny_terminals() {
        let mut app = App::new(Config::default());
        app.collections = (0..30)
            .map(|i| CollectionInfo::unloaded(format!("coll{}", i)))
            .collect();
        app.selected_collection = 29;
        app.documents = (0..30).map(|i| serde_json::json!({"_id": i})).collect();
        app.total_docs = 30;

        for width in 1..=24u16 {
            for height in 1..=24u16 {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| render_ui(f, &app)).unwrap();
            }
        }
    }
}
