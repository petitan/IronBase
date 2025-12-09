//! IronBase TUI - Intelligent Terminal Interface for IronBase NoSQL Database
//!
//! A pane-based, Lazygit-inspired TUI for browsing and querying IronBase databases.
//!
//! Uses MCP (Model Context Protocol) to communicate with IronBase.

mod app;
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
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
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

    /// MCP server URL (for HTTP transport)
    #[arg(long, short = 'u')]
    mcp_url: Option<String>,

    /// Use HTTP transport instead of stdio
    #[arg(long)]
    http: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load config
    let mut config = Config::load();

    // CLI overrides config
    if cli.http {
        config.transport = TransportMode::Http;
    }
    if let Some(url) = cli.mcp_url {
        config.mcp_url = url;
        config.transport = TransportMode::Http;
    }

    // Get db_path for later use
    let db_path = cli.database.or_else(|| config.last_db_path.clone());

    // Initialize app (without db yet, startup mode = splash screen)
    let mut app = App::new(config.clone());
    app.set_loading("Csatlakozas az adatbazishoz...");

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
            match DbWrapper::connect_http(&config.mcp_url).await {
                Ok(db) => {
                    app.db = Some(db);
                    app.db_path = db_path.clone();
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
                let server_path = config.get_mcp_server_path();
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

    // DB connection done - exit startup/splash mode
    app.startup = false;
    app.clear_loading();

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
        // Tick loading animation
        if app.is_loading() {
            app.tick_loading();
        }

        terminal.draw(|f| render_ui(f, app))?;

        // Use poll with timeout for non-blocking check
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    // Handle modal keys first
                    if app.modal.is_some() {
                        handle_modal_key_async(app, key.code, key.modifiers).await;
                    } else {
                        handle_global_key_async(app, key.code, key.modifiers).await;
                    }
                }
                Event::Resize(_, height) => {
                    // Update page_size when terminal is resized
                    if app.update_page_size(height) && !app.startup {
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
        KeyCode::Char('s') => {
            app.set_status("Save recipe - hamarosan...");
            app.close_modal();
        }
        KeyCode::Char('n') => {
            app.set_status("New collection - hamarosan...");
            app.close_modal();
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

    // Show splash screen during startup
    if app.startup {
        let msg = app.loading.as_deref().unwrap_or("Betoltes...");
        modals::splash::render(frame, frame.area(), msg, app.loading_frame, &theme);
        return;
    }

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
            Modal::ErrorDetail => {
                if let Some(ref err) = app.error_message {
                    modals::error::render(frame, frame.area(), err, app.error_scroll, &theme);
                }
            }
        }
    }
}

/// Render the header bar
fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let header_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // DB info
    let db_info = format!(
        " IronBase | {} ({} coll, {} docs) ",
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

    // Add loading indicator if loading
    if let Some(ref loading_msg) = app.loading {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled(
            format!("{} {}", app.loading_spinner(), loading_msg),
            Style::default().fg(theme.warning).bold(),
        ));
    } else {
        spans.push(Span::raw(" | "));
        spans.push(Span::styled("[/]", Style::default().fg(theme.accent)));
        spans.push(Span::raw(" Search "));
        spans.push(Span::styled("[?]", Style::default().fg(theme.accent)));
        spans.push(Span::raw(" Help"));
    }

    let line = Line::from(spans).patch_style(header_style);

    frame.render_widget(Paragraph::new(line), area);
}

/// Render the command bar at the bottom
fn render_command_bar(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    use ratatui::widgets::Paragraph;

    let bar_style = Style::default().bg(theme.header_bg).fg(theme.fg);

    // Context-dependent commands based on active pane
    let commands = match app.active_pane {
        Pane::Collections => vec![
            ("Tab", "Panel"),
            ("j/k", "Navigate"),
            ("Enter", "Select"),
            ("f", "Filter"),
            ("/", "Search"),
            ("a", "Actions"),
            ("q", "Quit"),
        ],
        Pane::Documents => vec![
            ("Tab", "Panel"),
            ("j/k", "Navigate"),
            ("PgUp/Dn", "Page"),
            ("f", "Filter"),
            ("/", "Search"),
            ("a", "Actions"),
            ("q", "Quit"),
        ],
        Pane::Detail => vec![
            ("Tab", "Panel"),
            ("j/k", "Scroll"),
            ("e", "Edit"),
            ("d", "Delete"),
            ("y", "Copy"),
            ("q", "Quit"),
        ],
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

        // Delete (Detail pane, Documents pane)
        (KeyCode::Char('d'), _)
            if app.active_pane == Pane::Detail || app.active_pane == Pane::Documents =>
        {
            app.open_delete_confirm();
        }

        // Insert (Collections, Documents pane)
        (KeyCode::Char('i'), _)
            if app.active_pane == Pane::Collections || app.active_pane == Pane::Documents =>
        {
            app.open_insert();
        }

        // Theme
        (KeyCode::Char('t'), _) => app.next_theme(),

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
            app.help_scroll = (app.help_scroll + 5).min(modals::help::HELP_LINES.saturating_sub(10));
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

/// Async search key handler
async fn handle_search_key_async(app: &mut App, key: KeyCode, _modifiers: KeyModifiers) {
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
            KeyCode::Char(c) => {
                app.search.insert_char(c);
            }
            KeyCode::Backspace => {
                app.search.delete_char();
            }
            KeyCode::Left => {
                app.search.cursor_left();
            }
            KeyCode::Right => {
                app.search.cursor_right();
            }
            KeyCode::Tab => {
                app.search.toggle_mode();
                app.search.results.clear();
                app.search.last_search.clear();
            }
            KeyCode::Down => {
                if !app.search.results.is_empty() {
                    app.search.input_active = false;
                }
            }
            _ => {}
        }
    } else {
        match key {
            KeyCode::Esc => app.close_modal(),
            KeyCode::Enter => {
                app.goto_search_result_async().await;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.search.selected_result == 0 {
                    app.search.input_active = true;
                } else {
                    app.search_select_up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.search_select_down();
            }
            KeyCode::Tab => {
                app.search.toggle_mode();
                app.execute_search_async().await;
            }
            // Mode shortcuts: 'c' for Collections, 'd' for Documents
            KeyCode::Char('c') => {
                if app.search.mode != crate::app::SearchMode::Collections {
                    app.search.mode = crate::app::SearchMode::Collections;
                    app.execute_search_async().await;
                }
            }
            KeyCode::Char('d') => {
                if app.search.mode != crate::app::SearchMode::Documents {
                    app.search.mode = crate::app::SearchMode::Documents;
                    app.execute_search_async().await;
                }
            }
            KeyCode::Char('/') => {
                app.search.input_active = true;
            }
            _ => {}
        }
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
        (KeyCode::F(5), _) => {
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
    match (key, modifiers) {
        (KeyCode::Esc, _) => app.close_modal(),
        (KeyCode::Char('s'), KeyModifiers::CONTROL) | (KeyCode::F(5), _) => {
            app.execute_query_async().await;
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            app.query_state.insert_newline();
        }
        (KeyCode::Tab, _) => {
            app.query_state.insert_tab();
        }
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

        // Execute filter search
        (KeyCode::F(5), _) => {
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
                    if app.filter_state.show_suggestions && !app.filter_state.filtered_suggestions.is_empty() {
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
                    let needs_value = app.filter_state.operator != crate::app::FilterOperator::Exists;
                    let has_value = !app.filter_state.value_input.is_empty();

                    if !needs_value || has_value {
                        app.filter_state.add_filter();
                    } else {
                        app.filter_state.error = Some("Ertek megadasa kotelezo!".to_string());
                    }
                }
                FilterFocus::Filters => {
                    // Filters-nél Enter = törölje a kiválasztott szűrőt
                    app.filter_state.remove_selected_filter();
                }
            }
        }

        // Left arrow - cursor left in text fields, prev operator in Operator focus
        (KeyCode::Left, _) => {
            match app.filter_state.focus {
                FilterFocus::Operator => app.filter_state.prev_operator(),
                FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_left(),
                _ => {}
            }
        }

        // Right arrow - cursor right in text fields, next operator in Operator focus
        (KeyCode::Right, _) => {
            match app.filter_state.focus {
                FilterFocus::Operator => app.filter_state.next_operator(),
                FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_right(),
                _ => {}
            }
        }

        // Home/End - cursor to start/end of text
        (KeyCode::Home, _) => app.filter_state.cursor_home(),
        (KeyCode::End, _) => app.filter_state.cursor_end(),

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
        (KeyCode::Char('d'), KeyModifiers::CONTROL) if app.filter_state.focus == FilterFocus::Filters => {
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
