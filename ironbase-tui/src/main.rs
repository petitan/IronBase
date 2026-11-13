//! IronBase TUI - Intelligent Terminal Interface for IronBase NoSQL Database
//!
//! A pane-based, Lazygit-inspired TUI for browsing and querying IronBase databases.
//!
//! Uses MCP (Model Context Protocol) to communicate with IronBase.

use clap::Parser;
use tuti_tui::app::{self, App};
use tuti_tui::config::{Config, TransportMode};
use tuti_tui::db::DbWrapper;
use tuti_tui::{handlers, ui};

// ============================================================================
// Flexible Boolean Parser for Environment Variables
// ============================================================================

/// Parse boolean from env var - accepts "1", "true", "yes", "on" (case-insensitive)
/// Empty string is treated as false (for unset env vars)
fn parse_bool_flexible(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(format!(
            "Invalid boolean value: '{}'. Use true/false/1/0/yes/no/on/off",
            s
        )),
    }
}
use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ironbase-tui")]
#[command(author, version, about = "Intelligent TUI for IronBase NoSQL database")]
struct Cli {
    /// Database file path (.mlite) - required for stdio transport
    #[arg()]
    database: Option<PathBuf>,

    /// MCP server URL for HTTP transport (e.g. --url https://localhost:8080/mcp)
    #[arg(long, visible_alias = "http")]
    url: Option<String>,

    /// API key for authentication
    #[arg(long, short = 'k', env = "IRONBASE_API_KEY")]
    api_key: Option<String>,

    /// Accept self-signed/invalid TLS certificates (for HTTPS with self-signed certs)
    #[arg(long, env = "MCP_INSECURE", value_parser = parse_bool_flexible, default_value = "false")]
    insecure: bool,

    /// MCP server executable path for stdio transport
    #[arg(long)]
    server: Option<PathBuf>,
}

/// Apply CLI/env overrides to the effective (runtime) config
fn apply_cli_overrides(config: &mut Config, cli: &Cli) {
    if let Some(url) = &cli.url {
        config.mcp_url = url.clone();
        config.transport = TransportMode::Http;
    }
    if let Some(key) = &cli.api_key {
        config.mcp_api_key = Some(key.clone());
    }
    if cli.insecure {
        config.mcp_insecure = true;
    }
    if cli.server.is_some() {
        config.transport = TransportMode::Stdio;
    }
}

/// The config to write on exit: the persisted one plus what the user changed
/// inside the TUI (theme, last database). CLI/env overrides live only in the
/// effective config (`app.config`), so a key or `--insecure` given on the
/// command line never reaches the file.
fn config_to_save(mut persisted: Config, app: &App) -> Config {
    persisted.theme = app.theme_name;
    persisted.last_db_path = app.db_path.clone();
    persisted
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Load the persisted config. A broken file is reported and never
    // overwritten on exit.
    let (persisted_config, config_error) = match Config::try_load() {
        Ok(config) => (config, None),
        Err(e) => {
            eprintln!(
                "Config hiba: {} - alapertelmezett beallitasokkal indul, a fajl nem lesz felulirva",
                e
            );
            (Config::default(), Some(e))
        }
    };

    // Effective runtime config: CLI/env overrides apply to this copy only and
    // are never written back to the config file
    let mut config = persisted_config.clone();
    apply_cli_overrides(&mut config, &cli);

    // Get db_path for later use
    let db_path = cli.database.or_else(|| config.last_db_path.clone());

    // Initialize app
    let mut app = App::new(config.clone());

    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before showing panic message
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Setup terminal FIRST - before DB connection (so we can show splash)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set page_size based on terminal size BEFORE loading documents
    let size = terminal.size()?;
    app.update_page_size(size.height);

    // Show splash screen immediately
    terminal.draw(|f| ui::render_ui(f, &app))?;

    // Connect to database (with splash screen visible)
    match config.transport {
        TransportMode::Http => {
            // HTTP transport - connect to external MCP server
            let api_key = config.get_mcp_api_key();
            match DbWrapper::connect_http_with_options(
                &config.mcp_url,
                api_key,
                config.mcp_insecure,
            )
            .await
            {
                Ok(db) => {
                    app.db = Some(db);
                    // Set connection type based on URL
                    app.connection_type = app::ConnectionType::from_url(&config.mcp_url);
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
                let server_path = cli
                    .server
                    .clone()
                    .unwrap_or_else(|| config.get_mcp_server_path());
                match DbWrapper::connect_stdio(&server_path, path).await {
                    Ok(db) => {
                        app.db = Some(db);
                        app.db_path = Some(path.clone());
                        // Stdio transport is always localhost
                        app.connection_type = app::ConnectionType::Localhost;
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
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Save config (only what the user changed inside the TUI)
    match config_error {
        None => {
            let _ = config_to_save(persisted_config, &app).save();
        }
        Some(e) => eprintln!("Config hiba: {} - a beallitasok nem lettek elmentve", e),
    }

    // Close MCP connection
    // In stdio mode this waits for the server's graceful shutdown (flush), which
    // can take a while on a large database: say so instead of sitting silent.
    if let Some(ref db) = app.db {
        let close = db.close();
        tokio::pin!(close);
        tokio::select! {
            _ = &mut close => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                eprintln!("Varakozas a szerver leallasara...");
                let _ = close.await;
            }
        }
    }

    result
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> anyhow::Result<()> {
    // Initial page_size update based on terminal size
    let size = terminal.size()?;
    app.update_page_size(size.height);

    loop {
        // Clear expired status messages
        app.clear_status_if_expired();

        terminal.draw(|f| ui::render_ui(f, app))?;

        // Use poll with timeout for non-blocking check
        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    // Windows sends both Press and Release events - only handle Press
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    // Modal keys first, global keys otherwise
                    handlers::handle_key(app, key).await;
                }
                Event::Resize(_, height)
                    // Update page_size when terminal is resized
                    if app.update_page_size(height) => {
                        // Refresh documents with new page size
                        app.reload_documents_async().await;
                    }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CLI/env overrides are runtime-only: a key or `--insecure` given on the
    /// command line used to be written to config.toml on exit
    #[test]
    fn test_cli_overrides_are_not_persisted() {
        let cli = Cli::parse_from([
            "tuti-tui",
            "--api-key",
            "sk-test",
            "--insecure",
            "--url",
            "https://db.example:9000/mcp",
        ]);
        let persisted = Config::default();
        let mut effective = persisted.clone();
        apply_cli_overrides(&mut effective, &cli);
        assert_eq!(effective.mcp_api_key.as_deref(), Some("sk-test"));
        assert!(effective.mcp_insecure);
        assert_eq!(effective.mcp_url, "https://db.example:9000/mcp");

        let mut app = App::new(effective);
        app.next_theme();
        app.db_path = Some(PathBuf::from("/data/x.mlite"));

        let saved = config_to_save(persisted.clone(), &app);
        assert_eq!(saved.mcp_api_key, None);
        assert!(!saved.mcp_insecure);
        assert_eq!(saved.mcp_url, persisted.mcp_url);
        assert_eq!(saved.transport, persisted.transport);
        // What the user changed inside the TUI is saved
        assert_eq!(saved.theme, app.theme_name);
        assert_ne!(saved.theme, persisted.theme);
        assert_eq!(saved.last_db_path, Some(PathBuf::from("/data/x.mlite")));
    }
}
