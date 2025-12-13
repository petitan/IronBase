//! IronBase TUI - Intelligent Terminal Interface for IronBase NoSQL Database
//!
//! A pane-based, Lazygit-inspired TUI for browsing and querying IronBase databases.
//!
//! Uses MCP (Model Context Protocol) to communicate with IronBase.

mod app;
mod base64_detect;
mod config;
mod db;
mod mcp;
mod modals;
mod panes;
mod theme;
mod widgets;

use app::{App, Modal, Pane};
use clap::Parser;
use config::{Config, TransportMode};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use db::DbWrapper;
use ratatui::prelude::*;
use std::io;
use std::path::PathBuf;
use theme::Theme;

#[derive(Parser)]
#[command(name = "ironbase-tui")]
#[command(author, version, about = "Intelligent TUI for IronBase NoSQL database")]
struct Cli {
    /// Database file path (.mlite) - required for stdio transport
    #[arg()]
    database: Option<PathBuf>,

    /// MCP server URL for HTTP transport (e.g. --http http://localhost:8080)
    #[arg(long)]
    http: Option<String>,

    /// MCP server executable path for stdio transport
    #[arg(long)]
    server: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load config
    let mut config = Config::load();

    // CLI overrides config
    if let Some(url) = cli.http {
        config.mcp_url = url;
        config.transport = TransportMode::Http;
    }
    if cli.server.is_some() {
        config.transport = TransportMode::Stdio;
    }

    // Get db_path for later use
    let db_path = cli.database.or_else(|| config.last_db_path.clone());

    // Initialize app
    let mut app = App::new(config.clone());

    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before showing panic message
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(panic_info);
    }));

    // Setup terminal FIRST - before DB connection (so we can show splash)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set page_size based on terminal size BEFORE loading documents
    let size = terminal.size()?;
    app.update_page_size(size.height);

    // Show splash screen immediately
    terminal.draw(|f| render_ui(f, &app))?;

    // Connect to database (with splash screen visible)
    match config.transport {
        TransportMode::Http => {
            // HTTP transport - connect to external MCP server
            let api_key = config.get_mcp_api_key();
            match DbWrapper::connect_http(&config.mcp_url, api_key).await {
                Ok(db) => {
                    app.db = Some(db);
                    // Fetch database path from MCP server
                    let _ = app.fetch_db_path_async().await;
                    if let Err(e) = app.refresh_collections_async().await {
                        app.set_error(format!("Nem sikerult betolteni: {}", e));
                    }
                }
                Err(e) => {
                    app.set_error(format!("MCP kapcsolat hiba ({}): {}", config.mcp_url, e));
                }
            }
        }
        TransportMode::Stdio => {
            // Stdio transport - spawn MCP server
            if let Some(ref path) = db_path {
                let server_path = cli.server.clone().unwrap_or_else(|| config.get_mcp_server_path());
                match DbWrapper::connect_stdio(&server_path, path).await {
                    Ok(db) => {
                        app.db = Some(db);
                        app.db_path = Some(path.clone());
                        if let Err(e) = app.refresh_collections_async().await {
                            app.set_error(format!("Nem sikerult betolteni: {}", e));
                        }
                    }
                    Err(e) => {
                        app.set_error(format!("MCP szerver inditasi hiba: {}", e));
                    }
                }
            } else {
                app.set_error("Stdio modban adatbazis utvonal szukseges (pl: ./data.mlite)");
            }
        }
    }


    // Run app
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Save config
    app.config.theme = app.theme_name;
    app.config.last_db_path = app.db_path.clone();
    let _ = app.config.save();

    // Close MCP connection
    if let Some(ref db) = app.db {
        let _ = db.close().await;
    }

    result
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> anyhow::Result<()> {
    // Initial page_size update based on terminal size
    let size = terminal.size()?;
    app.update_page_size(size.height);

    loop {
        terminal.draw(|f| render_ui(f, app))?;

        // Use poll with timeout for non-blocking check
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    // Windows sends both Press and Release events - only handle Press
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    // Handle modal keys first
                    if app.modal.is_some() {
                        handle_modal_key_async(app, key.code, key.modifiers).await;
                    } else {
                        handle_global_key_async(app, key.code, key.modifiers).await;
                    }
                }
                Event::Resize(_, height) => {
                    // Update page_size when terminal is resized
                    if app.update_page_size(height) {
                        // Refresh documents with new page size
                        let _ = app.refresh_documents_async().await;
                    }
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Handle keys in actions modal (async for schema loading)
async fn handle_actions_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
        // Action shortcuts
        KeyCode::Char('i') => {
            app.close_modal();
            app.open_insert();
        }
        KeyCode::Char('e') => {
            app.close_modal();
            app.open_edit();
        }
        KeyCode::Char('d') => {
            app.close_modal();
            app.open_delete_confirm();
        }
        KeyCode::Char('x') => {
            app.close_modal();
            app.open_index_modal();
            app.load_indexes_async().await;
        }
        KeyCode::Char('r') => {
            app.close_modal();
            app.open_query_modal();
        }
        KeyCode::Char('f') => {
            app.close_modal();
            app.open_filter_modal_async().await;
        }
        KeyCode::Char('o') => {
            app.close_modal();
            app.open_export_modal();
        }
        KeyCode::Char('c') => {
            app.close_modal();
            app.open_new_collection();
        }
        KeyCode::Char('y') => {
            app.close_modal();
            app.copy_document_to_clipboard();
        }
        KeyCode::Char('D') => {
            app.close_modal();
            app.open_delete_collection_confirm();
        }
        KeyCode::Char('s') => {
            app.close_modal();
            app.open_script_modal();
            app.load_scripts_async().await;
        }
        KeyCode::Char('b') => {
            app.close_modal();
            handle_database_open(app);
        }
        _ => {}
    }
}

/// Handle keys in error detail modal
fn handle_error_modal_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
            app.close_modal();
            app.error_message = None;
            app.error_scroll = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.error_scroll = app.error_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.error_scroll = app.error_scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            app.error_scroll = app.error_scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            app.error_scroll = app.error_scroll.saturating_add(10);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.error_scroll = 0;
        }
        _ => {}
    }
}

/// Handle keys in confirm dialog

fn render_ui(frame: &mut Frame, app: &App) {
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
        }
    }

}

/// Render the header bar
fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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

    // Build spans
    let mut spans = vec![
        Span::styled(db_info, Style::default().fg(theme.accent).bold()),
        Span::raw(" | "),
        Span::styled(pane_name, Style::default().fg(theme.fg)),
    ];

    let line = Line::from(spans).patch_style(header_style);

    frame.render_widget(Paragraph::new(line), area);
}

/// Render the command bar at the bottom
fn render_command_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let bar_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // Context-dependent commands based on active pane
    let commands: Vec<(&str, String)> = match app.active_pane {
        Pane::Collections => vec![
            ("Tab", "Panel".into()),
            ("j/k", "Navigate".into()),
            ("Enter", "Select".into()),
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
        // Truncate long errors for status bar display
        let max_err_len = 50;
        let truncated = if err.len() > max_err_len {
            format!("{}...", &err[..max_err_len])
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

// === Async key handlers ===
// These wrap the sync handlers and add async db operations where needed

/// Handle modal keys with async support
async fn handle_modal_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match app.modal {
        Some(Modal::Search) => handle_search_key_async(app, key, modifiers).await,
        Some(Modal::Actions) => handle_actions_key_async(app, key).await,
        Some(Modal::Help) => {
            handle_help_key(app, key);
        }
        Some(Modal::ErrorDetail) => handle_error_modal_key(app, key),
        Some(Modal::Confirm) => handle_confirm_key_async(app, key).await,
        Some(Modal::Insert) => handle_insert_key_async(app, key, modifiers).await,
        Some(Modal::Index) => handle_index_key_async(app, key, modifiers).await,
        Some(Modal::Query) => handle_query_key_async(app, key, modifiers).await,
        Some(Modal::Export) => handle_export_key_async(app, key, modifiers).await,
        Some(Modal::Filter) => handle_filter_key_async(app, key, modifiers).await,
        Some(Modal::NewCollection) => handle_new_collection_key_async(app, key).await,
        Some(Modal::Script) => handle_script_key_async(app, key, modifiers).await,
        Some(Modal::ServerInfo) => handle_server_info_key(app, key),
        Some(Modal::Update) => handle_update_key(app, key),
        Some(Modal::Database) => handle_database_key_async(app, key).await,
        Some(Modal::ApiKey) => handle_api_key_key_async(app, key).await,
        None => {}
    }
}

/// Handle global keys with async support
async fn handle_global_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        // Quit
        (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Char('q'), _) => {
            app.should_quit = true;
        }

        // Pane navigation
        (KeyCode::Tab, KeyModifiers::NONE) => app.next_pane(),
        (KeyCode::BackTab, _) => app.prev_pane(),

        // List navigation - Arrow keys
        (KeyCode::Up, _) => app.select_up_async().await,
        (KeyCode::Down, _) => app.select_down_async().await,
        (KeyCode::PageUp, _) => app.page_up_async().await,
        (KeyCode::PageDown, _) => app.page_down_async().await,
        (KeyCode::Home, _) => app.go_to_start_async().await,
        (KeyCode::End, _) => app.go_to_end_async().await,

        // List navigation - Vim keys
        (KeyCode::Char('k'), _) => app.select_up_async().await,
        (KeyCode::Char('j'), _) => app.select_down_async().await,
        (KeyCode::Char('g'), _) => app.go_to_start_async().await,
        (KeyCode::Char('G'), _) => app.go_to_end_async().await,

        // Modals
        (KeyCode::Char('/'), _) => app.open_search(),
        (KeyCode::Char('a'), _) => app.open_actions(),
        (KeyCode::Char('?'), _) => app.open_help(),
        (KeyCode::Char('I'), KeyModifiers::SHIFT) => handle_server_info_open(app).await,
        (KeyCode::Char('U'), KeyModifiers::SHIFT) => handle_update_open(app).await,
        (KeyCode::Char('f'), _) => app.open_filter_modal_async().await,

        // Error detail modal (if error present, 'e' opens error details)
        (KeyCode::Char('e'), _) if app.error_message.is_some() => {
            app.error_scroll = 0;
            app.modal = Some(Modal::ErrorDetail);
        }

        // Edit (Detail pane, Documents pane) - only if no error message
        (KeyCode::Char('e'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.open_edit();
        }

        // Delete document (Detail pane, Documents pane)
        (KeyCode::Char('d'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.open_delete_confirm();
        }

        // Delete collection (Collections pane - Shift+D)
        (KeyCode::Char('D'), KeyModifiers::SHIFT) if app.active_pane == Pane::Collections => {
            app.open_delete_collection_confirm();
        }

        // Insert (Collections, Documents pane)
        (KeyCode::Char('i'), _)
            if app.active_pane == Pane::Collections || app.active_pane == Pane::Documents =>
        {
            app.open_insert();
        }

        // Copy to clipboard (Detail or Documents pane)
        (KeyCode::Char('y'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.copy_document_to_clipboard();
        }

        // Search navigation (Detail pane - n/N for next/prev match)
        (KeyCode::Char('n'), _)
            if app.active_pane == Pane::Detail && !app.search.doc_matches.is_empty() =>
        {
            app.goto_next_match();
        }
        (KeyCode::Char('N'), KeyModifiers::SHIFT)
            if app.active_pane == Pane::Detail && !app.search.doc_matches.is_empty() =>
        {
            app.goto_prev_match();
        }

        // Theme
        (KeyCode::Char('t'), _) => app.next_theme(),

        // API Key management (Shift+K)
        (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
            app.api_key_state = modals::api_key::ApiKeyState::new();
            app.modal = Some(Modal::ApiKey);
            // Load API keys if admin key is available
            if let Some(ref admin_key) = app.api_key_state.admin_key {
                if let Some(ref db) = app.db {
                    app.api_key_state.loading = true;
                    match db.list_api_keys(admin_key).await {
                        Ok(keys) => {
                            let infos: Vec<_> = keys
                                .iter()
                                .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                .collect();
                            app.api_key_state.set_keys(infos);
                            app.api_key_state.loading = false;
                        }
                        Err(e) => {
                            app.api_key_state.set_error(format!("Failed to load keys: {}", e));
                            app.api_key_state.loading = false;
                        }
                    }
                }
            }
        }

        // Refresh (r) - reload collections and documents (detail updates automatically)
        (KeyCode::Char('r'), _) => {
            let _ = app.refresh_collections_async().await;
            let _ = app.refresh_documents_async().await;
        }

        // Clear filter (Escape in Documents pane when filter is active)
        (KeyCode::Esc, _) if app.active_filter.is_some() => {
            app.active_filter = None;
            app.doc_scroll_offset = 0;
            app.selected_document = 0;
            let _ = app.refresh_documents_async().await;
            app.set_status("Szuro torolve");
        }

        _ => {}
    }
}

/// Help modal key handler with scrolling support
fn handle_help_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.help_scroll = 0;
            app.close_modal();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.help_scroll < modals::help::HELP_LINES.saturating_sub(10) {
                app.help_scroll += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.help_scroll = app.help_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            app.help_scroll =
                (app.help_scroll + 5).min(modals::help::HELP_LINES.saturating_sub(10));
        }
        KeyCode::PageUp => {
            app.help_scroll = app.help_scroll.saturating_sub(5);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.help_scroll = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.help_scroll = modals::help::HELP_LINES.saturating_sub(10);
        }
        _ => {}
    }
}

/// Server info modal key handler with scrolling support
fn handle_server_info_key(app: &mut App, key: KeyCode) {
    let max_scroll = app.server_info_state.total_lines.saturating_sub(20);
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('I') => {
            app.server_info_scroll = 0;
            app.close_modal();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.server_info_scroll < max_scroll {
                app.server_info_scroll += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.server_info_scroll = app.server_info_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            app.server_info_scroll = (app.server_info_scroll + 10).min(max_scroll);
        }
        KeyCode::PageUp => {
            app.server_info_scroll = app.server_info_scroll.saturating_sub(10);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.server_info_scroll = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.server_info_scroll = max_scroll;
        }
        _ => {}
    }
}

/// Open server info modal and load data from MCP server
async fn handle_server_info_open(app: &mut App) {
    app.open_server_info();

    // Load data from MCP server
    if let Some(ref db) = app.db {
        // Fetch all three in parallel
        let (stats_result, tools_result, prompts_result) = tokio::join!(
            db.db_stats(),
            db.tools_list(),
            db.prompts_list()
        );

        match (stats_result, tools_result, prompts_result) {
            (Ok(stats), Ok(tools), Ok(prompts)) => {
                app.update_server_info(stats, tools, prompts);
            }
            (Err(e), _, _) => {
                app.set_server_info_error(format!("db_stats hiba: {}", e));
            }
            (_, Err(e), _) => {
                app.set_server_info_error(format!("tools_list hiba: {}", e));
            }
            (_, _, Err(e)) => {
                app.set_server_info_error(format!("prompts_list hiba: {}", e));
            }
        }
    } else {
        app.set_server_info_error("Nincs adatbazis kapcsolat".to_string());
    }
}

/// Handle update modal keys
fn handle_update_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('U') => {
            app.close_modal();
        }
        _ => {}
    }
}

/// Open update check modal and fetch latest version from GitHub
async fn handle_update_open(app: &mut App) {
    // Get current version from MCP server
    let current_version = if let Some(ref db) = app.db {
        match db.get_server_version().await {
            Ok(version) => version,
            Err(_) => "unknown".to_string(),
        }
    } else {
        "unknown".to_string()
    };

    app.open_update(current_version);

    // Fetch latest version from GitHub API
    match fetch_github_latest_release().await {
        Ok((latest_version, download_url, release_notes)) => {
            app.update_update_state(latest_version, download_url, release_notes);
        }
        Err(e) => {
            app.set_update_error(format!("GitHub API hiba: {}", e));
        }
    }
}

/// Fetch latest release info from GitHub
async fn fetch_github_latest_release() -> Result<(String, String, Option<String>), String> {
    let url = "https://api.github.com/repos/petitan/IronBase/releases/latest";

    let client = reqwest::Client::builder()
        .user_agent("IronBase-TUI")
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let tag_name = json.get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let html_url = json.get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/petitan/IronBase/releases")
        .to_string();

    let body = json.get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((tag_name, html_url, body))
}

/// Open database modal
fn handle_database_open(app: &mut App) {
    let is_http = app.config.transport == TransportMode::Http;
    let current_path = app.db_path.as_ref().map(|p| p.display().to_string());
    app.database_state = crate::app::DatabaseState::new(current_path.as_deref(), is_http);
    app.modal = Some(Modal::Database);
}

/// Handle database modal keys
async fn handle_database_key_async(app: &mut App, key: KeyCode) {
    use crate::app::DatabaseMode;

    match key {
        KeyCode::Esc => {
            app.close_modal();
        }
        KeyCode::Tab => {
            // Toggle between Open and Create mode
            app.database_state.toggle_mode();
        }
        KeyCode::Enter => {
            let path = app.database_state.path.trim().to_string();
            if path.is_empty() {
                app.database_state.error = Some("Az utvonal nem lehet ures!".to_string());
                return;
            }

            let create = app.database_state.mode == DatabaseMode::Create;

            // HTTP mode: use db_open MCP tool
            if app.database_state.is_http_mode {
                if let Some(ref db) = app.db {
                    app.database_state.loading = true;
                    app.database_state.error = None;

                    match db.db_open(&path, create).await {
                        Ok(()) => {
                            app.database_state.loading = false;
                            let action = if create { "letrehozva" } else { "megnyitva" };
                            app.database_state.message = Some(format!("Sikeresen {}!", action));

                            // Reset UI state and refresh collections
                            app.reset_for_new_database();
                            if let Err(e) = app.refresh_collections_async().await {
                                app.set_error(format!("Kollekcio betoltes hiba: {}", e));
                            }

                            // Close modal after success
                            app.close_modal();
                            app.set_status(format!("Adatbazis {}: {}", action, path));
                        }
                        Err(e) => {
                            app.database_state.loading = false;
                            app.database_state.error = Some(format!("{}", e));
                        }
                    }
                } else {
                    app.database_state.error = Some("Nincs aktiv kapcsolat!".to_string());
                }
                return;
            }

            // Stdio mode: restart MCP server with new database
            let path_buf = std::path::PathBuf::from(&path);
            let file_exists = path_buf.exists();

            // Validate based on mode
            match app.database_state.mode {
                DatabaseMode::Open => {
                    if !file_exists {
                        app.database_state.error = Some("A fajl nem letezik! Hasznald a 'Letrehozas' modot.".to_string());
                        return;
                    }
                }
                DatabaseMode::Create => {
                    if file_exists {
                        app.database_state.error = Some("A fajl mar letezik! Hasznald a 'Megnyitas' modot.".to_string());
                        return;
                    }
                }
            }

            app.database_state.loading = true;
            app.database_state.error = None;

            let server_path = app.config.get_mcp_server_path();

            // Close existing connection first
            if let Some(ref db) = app.db {
                let _ = db.close().await;
            }
            app.db = None;

            // Connect to database (MCP server will create if needed)
            match DbWrapper::connect_stdio(&server_path, &path_buf).await {
                Ok(db) => {
                    app.db = Some(db);
                    app.db_path = Some(path_buf);
                    app.database_state.loading = false;

                    let action = match app.database_state.mode {
                        DatabaseMode::Open => "megnyitva",
                        DatabaseMode::Create => "letrehozva",
                    };
                    app.database_state.message = Some(format!("Sikeresen {}!", action));

                    // Reset UI state and refresh collections
                    app.reset_for_new_database();
                    if let Err(e) = app.refresh_collections_async().await {
                        app.set_error(format!("Kollekcio betoltes hiba: {}", e));
                    }

                    // Close modal after success
                    app.close_modal();
                    app.set_status(format!("Adatbazis {}: {}", action, path));
                }
                Err(e) => {
                    app.database_state.loading = false;
                    app.database_state.error = Some(format!("Kapcsolodasi hiba: {}", e));
                }
            }
        }
        KeyCode::Char(c) => {
            app.database_state.insert_char(c);
        }
        KeyCode::Backspace => {
            app.database_state.backspace();
        }
        KeyCode::Delete => {
            app.database_state.delete();
        }
        KeyCode::Left => {
            app.database_state.move_left();
        }
        KeyCode::Right => {
            app.database_state.move_right();
        }
        KeyCode::Home => {
            app.database_state.home();
        }
        KeyCode::End => {
            app.database_state.end();
        }
        _ => {}
    }
}

/// Handle API Key modal keys
async fn handle_api_key_key_async(app: &mut App, key: KeyCode) {
    use crate::modals::api_key::{ApiKeyModalMode, ConfirmAction};

    match app.api_key_state.mode {
        ApiKeyModalMode::List => match key {
            KeyCode::Esc => {
                app.close_modal();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.api_key_state.select_next();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.api_key_state.select_prev();
            }
            KeyCode::Char('n') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_create();
            }
            KeyCode::Char('r') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_revoke();
            }
            KeyCode::Char('d') if app.api_key_state.has_admin_key() => {
                app.api_key_state.start_delete();
            }
            KeyCode::Char('c') => {
                // Copy new key to clipboard (if available)
                if let Some(ref key) = app.api_key_state.new_key {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if clipboard.set_text(key.clone()).is_ok() {
                            app.api_key_state.set_success("Key copied to clipboard!".to_string());
                        }
                    }
                }
            }
            _ => {}
        },
        ApiKeyModalMode::Create => match key {
            KeyCode::Esc => {
                app.api_key_state.cancel();
            }
            KeyCode::Enter => {
                let name = app.api_key_state.input.trim().to_string();
                if name.is_empty() {
                    app.api_key_state.set_error("Name cannot be empty".to_string());
                    return;
                }

                if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                    if let Some(ref db) = app.db {
                        match db.create_api_key(&admin_key, &name).await {
                            Ok(result) => {
                                // Extract the full key from the result
                                if let Some(key) = result.get("key").and_then(|v| v.as_str()) {
                                    app.api_key_state.new_key = Some(key.to_string());
                                    // Save key to config directory for easy copying
                                    if let Some(config_dir) = dirs::config_dir() {
                                        let tui_dir = config_dir.join("ironbase-tui");
                                        let _ = std::fs::create_dir_all(&tui_dir);
                                        let key_file = tui_dir.join("new_key.txt");
                                        let _ = std::fs::write(&key_file, format!("{}\n", key));
                                        // Set restrictive permissions (owner read/write only)
                                        #[cfg(unix)]
                                        {
                                            use std::os::unix::fs::PermissionsExt;
                                            let _ = std::fs::set_permissions(
                                                &key_file,
                                                std::fs::Permissions::from_mode(0o600),
                                            );
                                        }
                                    }
                                }
                                app.api_key_state.set_success(format!("API key '{}' created! Saved to ~/.config/ironbase-tui/new_key.txt", name));
                                app.api_key_state.mode = ApiKeyModalMode::List;
                                app.api_key_state.input.clear();

                                // Refresh the keys list
                                if let Ok(keys) = db.list_api_keys(&admin_key).await {
                                    let infos: Vec<_> = keys
                                        .iter()
                                        .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                        .collect();
                                    app.api_key_state.set_keys(infos);
                                }
                            }
                            Err(e) => {
                                app.api_key_state.set_error(format!("Failed to create: {}", e));
                            }
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                app.api_key_state.input.push(c);
            }
            KeyCode::Backspace => {
                app.api_key_state.input.pop();
            }
            _ => {}
        },
        ApiKeyModalMode::Confirm => match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                app.api_key_state.cancel();
            }
            KeyCode::Char('y') => {
                if let Some(selected) = app.api_key_state.get_selected() {
                    let id = selected.id;
                    let name = selected.name.clone();

                    if let Some(ref admin_key) = app.api_key_state.admin_key.clone() {
                        if let Some(ref db) = app.db {
                            let result = match app.api_key_state.confirm_action {
                                Some(ConfirmAction::Revoke) => {
                                    db.revoke_api_key(&admin_key, id).await
                                }
                                Some(ConfirmAction::Delete) => {
                                    db.delete_api_key(&admin_key, id).await
                                }
                                None => Ok(false),
                            };

                            match result {
                                Ok(true) => {
                                    let action = match app.api_key_state.confirm_action {
                                        Some(ConfirmAction::Revoke) => "revoked",
                                        Some(ConfirmAction::Delete) => "deleted",
                                        None => "modified",
                                    };
                                    app.api_key_state.set_success(format!("Key '{}' {}!", name, action));
                                    app.api_key_state.mode = ApiKeyModalMode::List;
                                    app.api_key_state.confirm_action = None;

                                    // Refresh the keys list
                                    if let Ok(keys) = db.list_api_keys(&admin_key).await {
                                        let infos: Vec<_> = keys
                                            .iter()
                                            .filter_map(modals::api_key::ApiKeyInfo::from_value)
                                            .collect();
                                        app.api_key_state.set_keys(infos);
                                    }
                                }
                                Ok(false) => {
                                    app.api_key_state.set_error("Key not found".to_string());
                                    app.api_key_state.cancel();
                                }
                                Err(e) => {
                                    app.api_key_state.set_error(format!("Failed: {}", e));
                                    app.api_key_state.cancel();
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        },
    }
}

/// Async search key handler
async fn handle_search_key_async(app: &mut App, key: KeyCode, _modifiers: KeyModifiers) {
    use crate::app::SearchMode;

    match app.search.mode {
        SearchMode::Collections => handle_collection_search_key(app, key).await,
        SearchMode::Document => handle_document_search_key(app, key),
    }
}

/// Handle collection search keys
async fn handle_collection_search_key(app: &mut App, key: KeyCode) {
    if app.search.input_active {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Enter => {
                if app.search.query_input.is_empty() {
                    app.close_modal();
                } else {
                    app.execute_search_async().await;
                    if !app.search.results.is_empty() {
                        app.search.input_active = false;
                    }
                }
            }
            KeyCode::Char(c) => app.search.insert_char(c),
            KeyCode::Backspace => app.search.delete_char(),
            KeyCode::Left => app.search.cursor_left(),
            KeyCode::Right => app.search.cursor_right(),
            KeyCode::Tab | KeyCode::Down => {
                if !app.search.results.is_empty() {
                    app.search.input_active = false;
                }
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Enter => app.goto_search_result_async().await,
            KeyCode::Up | KeyCode::Char('k') => {
                if app.search.selected_result == 0 {
                    app.search.input_active = true;
                } else {
                    app.search_select_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => app.search_select_down(),
            KeyCode::Tab | KeyCode::Char('/') => app.search.input_active = true,
            _ => {}
        }
    }
}

/// Handle document search keys
fn handle_document_search_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Enter => {
            if !app.search.query_input.is_empty() {
                app.execute_document_search();
                // Ha van találat, zárd be a modalt és váltsd Detail pane-re
                if !app.search.doc_matches.is_empty() {
                    app.close_modal();
                    app.active_pane = crate::app::Pane::Detail;
                }
            }
        }
        KeyCode::Char('n') => {
            // Next match
            if !app.search.doc_matches.is_empty() {
                app.goto_next_match();
            } else {
                app.search.insert_char('n');
            }
        }
        KeyCode::Char('N') => {
            // Previous match
            if !app.search.doc_matches.is_empty() {
                app.goto_prev_match();
            } else {
                app.search.insert_char('N');
            }
        }
        KeyCode::Char(c) => app.search.insert_char(c),
        KeyCode::Backspace => app.search.delete_char(),
        KeyCode::Left => app.search.cursor_left(),
        KeyCode::Right => app.search.cursor_right(),
        _ => {}
    }
}

/// Async confirm key handler
async fn handle_confirm_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h') | KeyCode::Char('l') => {
            app.confirm_toggle();
        }
        KeyCode::Enter => {
            app.execute_confirm_action_async().await;
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.confirm.selected = crate::modals::confirm::ConfirmOption::Confirm;
            app.execute_confirm_action_async().await;
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            app.close_modal();
        }
        _ => {}
    }
}

/// Async insert key handler
async fn handle_insert_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_insert_async().await;
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            app.insert.insert_newline();
        }
        (KeyCode::Tab, _) => {
            app.insert.insert_tab();
        }
        (KeyCode::Backspace, _) => {
            app.insert.backspace();
        }
        (KeyCode::Left, _) => {
            app.insert.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.insert.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.insert.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.insert.cursor_down();
        }
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.insert.insert_char(c);
        }
        _ => {}
    }
}

/// Async index key handler
async fn handle_index_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if app.index_state.is_creating {
        match (key, modifiers) {
            (KeyCode::Esc, _) => {
                app.index_state.cancel_create();
            }
            (KeyCode::Enter, _) => {
                app.execute_create_index_async().await;
            }
            (KeyCode::Tab, _) => {
                app.index_state.next_form_field();
            }
            (KeyCode::Char(' '), _) if app.index_state.form_field == 1 => {
                app.index_state.toggle_unique();
            }
            (KeyCode::Backspace, _) if app.index_state.form_field == 0 => {
                app.index_state.backspace();
            }
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT)
                if app.index_state.form_field == 0 =>
            {
                app.index_state.insert_char(c);
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Up | KeyCode::Char('k') => app.index_state.select_up(),
            KeyCode::Down | KeyCode::Char('j') => app.index_state.select_down(),
            KeyCode::Tab | KeyCode::Char('n') => app.index_state.start_create(),
            KeyCode::Char('d') | KeyCode::Delete => app.execute_delete_index_async().await,
            _ => {}
        }
    }
}

/// Async query key handler
async fn handle_query_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    // Handle template mode separately
    if app.query_state.show_templates {
        match key {
            KeyCode::Esc => app.query_state.show_templates = false,
            KeyCode::Tab => app.query_state.show_templates = false,
            KeyCode::Up | KeyCode::Char('k') => app.query_state.template_up(),
            KeyCode::Down | KeyCode::Char('j') => app.query_state.template_down(),
            KeyCode::Enter => app.query_state.apply_template(),
            _ => {}
        }
        return;
    }

    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        // Ctrl+S = Run query
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_query_async().await;
        }
        // Enter = Apply filter (ha van eredmény) VAGY newline (ha nincs)
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if app.query_state.has_results() {
                app.apply_query_as_filter().await;
            } else {
                app.query_state.insert_newline();
            }
        }
        // Tab = Insert 2 spaces
        (KeyCode::Tab, _) => {
            app.query_state.insert_tab();
        }
        // Ctrl+T = Templates
        (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
            app.query_state.toggle_templates();
        }
        // Ctrl+j/k = Navigate results
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
            app.query_state.result_down();
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            app.query_state.result_up();
        }
        // Arrow keys = cursor movement
        (KeyCode::Backspace, _) => {
            app.query_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.query_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.query_state.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.query_state.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.query_state.cursor_down();
        }
        // All other chars = insert
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.query_state.insert_char(c);
        }
        _ => {}
    }
}

/// Async export key handler
async fn handle_export_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::modals::export::ExportFormat;

    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Enter, _) => {
            app.execute_export_async().await;
        }
        (KeyCode::Tab, _) => {
            app.export_state.toggle_format();
        }
        (KeyCode::Char('J'), KeyModifiers::SHIFT) => {
            app.export_state.set_format(ExportFormat::Json);
        }
        (KeyCode::Char('C'), KeyModifiers::SHIFT) => {
            app.export_state.set_format(ExportFormat::Csv);
        }
        (KeyCode::Backspace, _) => {
            app.export_state.backspace();
        }
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.export_state.insert_char(c);
        }
        _ => {}
    }
}

/// Async filter key handler
async fn handle_filter_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::FilterFocus;

    match (key, modifiers) {
        // Close modal
        (KeyCode::Esc, _) => app.close_modal(),

        // Execute filter search (Ctrl+S)
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_filter_async().await;
        }

        // Tab - apply suggestion if showing, otherwise cycle between fields
        (KeyCode::Tab, KeyModifiers::NONE) => {
            if app.filter_state.focus == FilterFocus::Field && app.filter_state.show_suggestions {
                // Apply suggestion and move to operator
                app.filter_state.apply_suggestion();
                app.filter_state.focus = FilterFocus::Operator;
            } else {
                app.filter_state.next_focus();
            }
        }
        (KeyCode::BackTab, _) => {
            app.filter_state.prev_focus();
        }

        // Enter - navigate or add filter
        (KeyCode::Enter, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Ha van suggestion látható, alkalmazzuk és lépjünk tovább
                    if app.filter_state.show_suggestions
                        && !app.filter_state.filtered_suggestions.is_empty()
                    {
                        app.filter_state.apply_suggestion();
                        app.filter_state.focus = FilterFocus::Operator;
                    } else if !app.filter_state.field_input.is_empty() {
                        // Field-nél Enter = lépj Operator-ra, DE csak ha nem üres!
                        app.filter_state.focus = FilterFocus::Operator;
                    }
                }
                FilterFocus::Operator => {
                    // Operator-nál Enter = lépj Value-ra
                    app.filter_state.focus = FilterFocus::Value;
                }
                FilterFocus::Value => {
                    // Value-nál Enter = add filter (ha valid) és vissza Field-re
                    // Exists operátorhoz nem kell érték, máshoz igen
                    let needs_value =
                        app.filter_state.operator != crate::app::FilterOperator::Exists;
                    let has_value = !app.filter_state.value_input.is_empty();

                    if !needs_value || has_value {
                        app.filter_state.add_filter();
                    } else {
                        app.filter_state.error = Some("Ertek megadasa kotelezo!".to_string());
                    }
                }
                FilterFocus::Filters => {
                    // Filters-nél Enter = szerkesztés (Delete = törlés)
                    app.filter_state.edit_selected_filter();
                }
                FilterFocus::SortField => {
                    // SortField-nél Enter = tovább a Field-re
                    app.filter_state.next_focus();
                }
            }
        }

        // Left arrow - cursor left in text fields, prev operator in Operator focus, prev sort field
        (KeyCode::Left, _) => match app.filter_state.focus {
            FilterFocus::Operator => app.filter_state.prev_operator(),
            FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_left(),
            FilterFocus::SortField => app.filter_state.prev_sort_field(),
            _ => {}
        },

        // Right arrow - cursor right in text fields, next operator in Operator focus, next sort field
        (KeyCode::Right, _) => match app.filter_state.focus {
            FilterFocus::Operator => app.filter_state.next_operator(),
            FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_right(),
            FilterFocus::SortField => app.filter_state.next_sort_field(),
            _ => {}
        },

        // Space - toggle sort direction in SortField focus
        (KeyCode::Char(' '), _) if app.filter_state.focus == FilterFocus::SortField => {
            app.filter_state.toggle_sort_direction();
        }

        // Home/End - cursor to start/end of text
        (KeyCode::Home, _) => app.filter_state.cursor_home(),
        (KeyCode::End, _) => app.filter_state.cursor_end(),

        // Ctrl+Up/Down - move filter up/down in list
        (KeyCode::Up, KeyModifiers::CONTROL) if app.filter_state.focus == FilterFocus::Filters => {
            app.filter_state.move_filter_up();
        }
        (KeyCode::Down, KeyModifiers::CONTROL)
            if app.filter_state.focus == FilterFocus::Filters =>
        {
            app.filter_state.move_filter_down();
        }

        // Up/Down - navigate suggestions (Field) or filters list (Filters)
        (KeyCode::Up, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Navigate suggestions up
                    if app.filter_state.show_suggestions {
                        app.filter_state.suggestion_up();
                    } else {
                        // Show suggestions if we have them
                        app.filter_state.update_suggestions();
                    }
                }
                FilterFocus::Filters => {
                    if app.filter_state.selected_filter > 0 {
                        app.filter_state.selected_filter -= 1;
                    }
                }
                _ => {}
            }
        }
        (KeyCode::Down, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Navigate suggestions down
                    if app.filter_state.show_suggestions {
                        app.filter_state.suggestion_down();
                    } else {
                        // Show suggestions if we have them
                        app.filter_state.update_suggestions();
                    }
                }
                FilterFocus::Filters => {
                    if app.filter_state.selected_filter + 1 < app.filter_state.filters.len() {
                        app.filter_state.selected_filter += 1;
                    }
                }
                _ => {}
            }
        }

        // Delete - remove selected filter
        (KeyCode::Delete, _) if app.filter_state.focus == FilterFocus::Filters => {
            app.filter_state.remove_selected_filter();
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL)
            if app.filter_state.focus == FilterFocus::Filters =>
        {
            app.filter_state.remove_selected_filter();
        }

        // Backspace - delete character
        (KeyCode::Backspace, _) => {
            app.filter_state.backspace();
        }

        // Character input - only for Field and Value focus
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match app.filter_state.focus {
                FilterFocus::Field | FilterFocus::Value => {
                    app.filter_state.insert_char(c);
                }
                _ => {}
            }
        }

        _ => {}
    }
}

/// Async new collection key handler
async fn handle_new_collection_key_async(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc => app.close_modal(),
        KeyCode::Enter => {
            app.create_collection_async().await;
        }
        KeyCode::Backspace => {
            app.new_collection_state.backspace();
        }
        KeyCode::Left => {
            app.new_collection_state.cursor_left();
        }
        KeyCode::Right => {
            app.new_collection_state.cursor_right();
        }
        KeyCode::Char(c) => {
            app.new_collection_state.insert_char(c);
        }
        _ => {}
    }
}

/// Async script key handler
async fn handle_script_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::{ScriptConfirmAction, ScriptMode};

    // Handle confirmation dialog first if active
    if let Some(action) = app.script_state.confirm_action {
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.script_state.confirm_action = None;
                match action {
                    ScriptConfirmAction::DiscardChanges => {
                        app.script_state.dirty = false;
                        app.script_state.reset_to_browse();
                    }
                    ScriptConfirmAction::DeleteScript => {
                        app.delete_script_async().await;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.script_state.confirm_action = None;
            }
            _ => {}
        }
        return;
    }

    match app.script_state.mode {
        ScriptMode::Browse => handle_script_browse_key(app, key, modifiers).await,
        ScriptMode::Edit | ScriptMode::New | ScriptMode::Inline => {
            handle_script_edit_key(app, key, modifiers).await
        }
        ScriptMode::History => handle_script_history_key(app, key).await,
    }
}

/// Handle keys in Browse mode (script list)
async fn handle_script_browse_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => app.close_modal(),
        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE) => {
            if app.script_state.selected_script > 0 {
                app.script_state.selected_script -= 1;
            }
        }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => {
            if app.script_state.selected_script + 1 < app.script_state.scripts.len() {
                app.script_state.selected_script += 1;
            }
        }
        (KeyCode::Enter, _) => {
            // Open selected script for editing (load full script from MCP)
            if !app.script_state.scripts.is_empty() {
                app.load_script_for_edit_async().await;
            }
        }
        (KeyCode::Char('n'), _) => {
            // New script
            app.script_state.start_new();
        }
        (KeyCode::Char('d'), _) | (KeyCode::Delete, _) => {
            // Delete selected script (with confirmation)
            if !app.script_state.scripts.is_empty() {
                app.script_state.confirm_action = Some(crate::app::ScriptConfirmAction::DeleteScript);
            }
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            // Run selected script directly from browse (Ctrl+S)
            if !app.script_state.scripts.is_empty() {
                // Load the script first, then run
                app.load_script_for_edit_async().await;
                app.run_script_async().await;
            }
        }
        (KeyCode::Char('h'), _) => {
            // Show history for selected script
            if !app.script_state.scripts.is_empty() {
                // First load the script to get the name
                app.load_script_for_edit_async().await;
                // Then load history
                app.load_script_history_async().await;
                app.script_state.enter_history();
            }
        }
        (KeyCode::Char('i'), _) => {
            // Inline mode (ad-hoc script)
            app.script_state.enter_inline();
        }
        (KeyCode::Char('r'), _) => {
            // Refresh script list
            app.load_scripts_async().await;
        }
        _ => {}
    }
}

/// Handle keys in Edit/New/Inline mode
async fn handle_script_edit_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::ScriptFocus;

    match (key, modifiers) {
        // Exit/Back
        (KeyCode::Esc, _) => {
            if app.script_state.dirty {
                // Ask for confirmation before discarding changes
                app.script_state.confirm_action = Some(crate::app::ScriptConfirmAction::DiscardChanges);
            } else {
                app.script_state.reset_to_browse();
            }
        }

        // Save & Run (Ctrl+S)
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            // Save first (if not inline)
            if app.script_state.mode != crate::app::ScriptMode::Inline {
                app.save_script_async().await;
                app.load_scripts_async().await;
            }
            // Then run
            app.run_script_async().await;
        }

        // Tab - cycle focus
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.script_state.next_focus();
        }
        (KeyCode::BackTab, _) => {
            app.script_state.prev_focus();
        }

        // Focus-specific handling
        _ => {
            match app.script_state.focus {
                ScriptFocus::Name => handle_script_name_key(app, key, modifiers),
                ScriptFocus::Description => handle_script_desc_key(app, key, modifiers),
                ScriptFocus::Tags => handle_script_tags_key(app, key, modifiers),
                ScriptFocus::Editor => handle_script_editor_key(app, key, modifiers),
                ScriptFocus::Params => handle_script_params_key(app, key, modifiers),
                _ => {}
            }
        }
    }
}

/// Handle keys in History mode
async fn handle_script_history_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Back to edit mode
            app.script_state.mode = crate::app::ScriptMode::Edit;
            app.script_state.focus = crate::app::ScriptFocus::Editor;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.script_state.selected_version > 0 {
                app.script_state.selected_version -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.script_state.selected_version + 1 < app.script_state.versions.len() {
                app.script_state.selected_version += 1;
            }
        }
        KeyCode::Enter => {
            // Load selected version into editor (view mode)
            if let Some(version) = app.script_state.versions.get(app.script_state.selected_version) {
                app.script_state.lines = version.code.lines().map(String::from).collect();
                if app.script_state.lines.is_empty() {
                    app.script_state.lines.push(String::new());
                }
                app.script_state.cursor_line = 0;
                app.script_state.cursor_col = 0;
                app.script_state.scroll_offset = 0;
                app.script_state.mode = crate::app::ScriptMode::Edit;
                app.script_state.focus = crate::app::ScriptFocus::Editor;
                app.script_state.message = Some(format!("Verzió v{} betöltve (readonly)", version.version));
                app.script_state.dirty = true; // Mark as dirty so user knows it's not the current version
            }
        }
        KeyCode::Char('r') => {
            // Rollback to selected version
            if !app.script_state.versions.is_empty() {
                app.rollback_script_async().await;
            }
        }
        _ => {}
    }
}

/// Handle name field keys
fn handle_script_name_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Enter, _) => {
            app.script_state.next_focus();
        }
        _ => {}
    }
}

/// Handle description field keys
fn handle_script_desc_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Enter, _) => {
            app.script_state.next_focus();
        }
        _ => {}
    }
}

/// Handle tags field keys (chips editor)
fn handle_script_tags_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if app.script_state.tag_input_active {
        // Typing new tag
        match (key, modifiers) {
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                app.script_state.tag_input.push(c);
            }
            (KeyCode::Backspace, _) => {
                if app.script_state.tag_input.is_empty() {
                    app.script_state.tag_input_active = false;
                } else {
                    app.script_state.tag_input.pop();
                }
            }
            (KeyCode::Enter, _) => {
                app.script_state.add_tag();
            }
            (KeyCode::Esc, _) => {
                app.script_state.tag_input_active = false;
                app.script_state.tag_input.clear();
            }
            _ => {}
        }
    } else {
        // Navigating tags
        match key {
            KeyCode::Left | KeyCode::Char('h') => {
                if app.script_state.selected_tag > 0 {
                    app.script_state.selected_tag -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                // +1 for the [+] button
                if app.script_state.selected_tag < app.script_state.tags.len() {
                    app.script_state.selected_tag += 1;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if app.script_state.selected_tag == app.script_state.tags.len() {
                    // [+] button - start adding new tag
                    app.script_state.tag_input_active = true;
                }
            }
            KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x') => {
                // Delete selected tag
                if app.script_state.selected_tag < app.script_state.tags.len() {
                    app.script_state.tags.remove(app.script_state.selected_tag);
                    if app.script_state.selected_tag > 0
                        && app.script_state.selected_tag >= app.script_state.tags.len()
                    {
                        app.script_state.selected_tag = app.script_state.tags.len();
                    }
                    app.script_state.dirty = true;
                }
            }
            _ => {}
        }
    }
}

/// Handle editor (code) keys
fn handle_script_editor_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Enter, _) => {
            app.script_state.insert_newline();
        }
        (KeyCode::Tab, KeyModifiers::NONE) => {
            // Insert 4 spaces as tab in editor
            app.script_state.insert_tab();
            app.script_state.insert_tab();
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Delete, _) => {
            app.script_state.delete_char();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.script_state.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.script_state.cursor_down();
        }
        (KeyCode::Home, _) => {
            app.script_state.cursor_home();
        }
        (KeyCode::End, _) => {
            app.script_state.cursor_end();
        }
        (KeyCode::PageUp, _) => {
            for _ in 0..10 {
                app.script_state.cursor_up();
            }
        }
        (KeyCode::PageDown, _) => {
            for _ in 0..10 {
                app.script_state.cursor_down();
            }
        }
        _ => {}
    }
}

/// Handle params field keys
fn handle_script_params_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        _ => {}
    }
}
