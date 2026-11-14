//! IronBase TUI - Intelligent Terminal Interface for IronBase NoSQL Database
//!
//! A pane-based, Lazygit-inspired TUI for browsing and querying IronBase databases.
//!
//! Uses MCP (Model Context Protocol) to communicate with IronBase.

use clap::Parser;
use tuti_tui::app::{self, App};
use tuti_tui::config::{Config, TransportMode};
use tuti_tui::db::DbWrapper;
use tuti_tui::{handlers, logging, ui};

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
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;
use std::io::IsTerminal;
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

    /// Log file path (default: state/data dir, ironbase-tui/tuti-tui.log)
    #[arg(long)]
    log_file: Option<PathBuf>,
}

/// The TUI needs an interactive terminal on stdout: refuse to start
/// otherwise (e.g. `tuti-tui | cat`) instead of failing in raw-mode setup
fn require_terminal(stdout_is_terminal: bool) -> Result<(), String> {
    if stdout_is_terminal {
        Ok(())
    } else {
        Err("tuti-tui: a kimenet (stdout) nem terminal - interaktiv terminal szukseges".to_string())
    }
}

/// Start file logging. Never fails startup: without a usable log file the
/// TUI runs unlogged, and says so once (before the TUI takes the terminal).
fn init_logging(cli: &Cli) {
    let level = match logging::parse_level(std::env::var(logging::LOG_LEVEL_ENV).ok().as_deref()) {
        Ok(level) => level,
        Err(e) => {
            eprintln!("Log hiba: {} - info szinttel indul", e);
            tracing_subscriber::filter::LevelFilter::INFO
        }
    };
    let result = match cli.log_file.clone().or_else(logging::default_log_path) {
        Some(path) => logging::init(&path, level),
        None => Err(anyhow::anyhow!("No state directory")),
    };
    if let Err(e) = result {
        eprintln!("Log hiba: {} - naplozas nelkul indul", e);
    }
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
///
/// The remembered database is replaced only by one that was opened: a
/// session that never started (terminal setup failed) or never connected
/// keeps the persisted value.
fn config_to_save(mut persisted: Config, app: &App) -> Config {
    persisted.theme = app.theme_name;
    if let Some(path) = &app.db_path {
        persisted.last_db_path = Some(path.clone());
    }
    persisted
}

/// The key given on the command line / in the environment: known before
/// the config is read, so it is masked in the log from the first line. (The
/// admin key is registered where it is read: `ApiKeyState::new`.)
fn register_startup_secrets(cli: &Cli) {
    if let Some(key) = &cli.api_key {
        logging::register_secret(key);
    }
}

/// The keys of the persisted and of the effective config (the latter falls
/// back to the environment)
fn register_config_secrets(persisted: &Config, effective: &Config) {
    for key in [persisted.mcp_api_key.clone(), effective.get_mcp_api_key()]
        .into_iter()
        .flatten()
    {
        logging::register_secret(&key);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Err(e) = require_terminal(std::io::stdout().is_terminal()) {
        eprintln!("{}", e);
        std::process::exit(2);
    }

    init_logging(&cli);
    register_startup_secrets(&cli);

    // Load the persisted config. A broken file is reported and never
    // overwritten on exit.
    let (persisted_config, config_error) = match Config::try_load() {
        Ok(config) => {
            tracing::info!("Config loaded");
            (config, None)
        }
        Err(e) => {
            tracing::warn!(
                error = %logging::sanitize_for_log(&e.to_string()),
                "Config load failed, using defaults"
            );
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
    register_config_secrets(&persisted_config, &config);

    // Never the full URL: it can carry credentials
    match config.transport {
        TransportMode::Http => tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            transport = "http",
            host = %logging::url_host(&config.mcp_url),
            "tuti-tui starting"
        ),
        TransportMode::Stdio => tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            transport = "stdio",
            "tuti-tui starting"
        ),
    }

    // Get db_path for later use
    let db_path = cli.database.or_else(|| config.last_db_path.clone());

    // Initialize app
    let mut app = App::new(config.clone());

    // Setup terminal FIRST - before DB connection (so we can show splash).
    // Raw mode + alternate screen, no mouse capture; also installs a panic
    // hook that restores the terminal. From here to `try_restore` nothing
    // returns early: every error lands in `result`.
    let result = match ratatui::try_init() {
        Ok(mut terminal) => {
            let result = run_session(&mut terminal, &mut app, &config, db_path, cli.server).await;
            let _ = terminal.show_cursor();
            result
        }
        Err(e) => Err(e.into()),
    };

    // Restore terminal - also after a failed setup (raw mode can be on)
    // and after an error in the session
    let restored = ratatui::try_restore();
    if let Err(e) = &result {
        tracing::error!(error = %logging::sanitize_for_log(&e.to_string()), "TUI session failed");
    }
    if let Err(e) = &restored {
        tracing::error!(
            error = %logging::sanitize_for_log(&e.to_string()),
            "Terminal restore failed"
        );
    }

    // Save config (only what the user changed inside the TUI)
    match config_error {
        None => match config_to_save(persisted_config, &app).save() {
            Ok(()) => tracing::info!("Config saved"),
            Err(e) => tracing::warn!(
                error = %logging::sanitize_for_log(&e.to_string()),
                "Config save failed"
            ),
        },
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

    tracing::info!("tuti-tui shutdown");

    result.and(restored.map_err(Into::into))
}

/// Everything that runs while the TUI owns the terminal: splash, connect,
/// event loop. The caller restores the terminal whatever this returns.
async fn run_session(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    config: &Config,
    db_path: Option<PathBuf>,
    server: Option<PathBuf>,
) -> anyhow::Result<()> {
    // Set page_size based on terminal size BEFORE loading documents
    let size = terminal.size()?;
    app.update_page_size(size.height);

    // Show splash screen immediately
    terminal.draw(|f| ui::render_ui(f, app))?;

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
                let server_path = server.unwrap_or_else(|| config.get_mcp_server_path());
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
    run_app(terminal, app).await
}

async fn run_app(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
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

    /// A session that never started (terminal setup failed) or never
    /// connected has no database path: the remembered one used to be erased
    #[test]
    fn test_last_db_path_survives_a_session_without_database() {
        let persisted = Config {
            last_db_path: Some(PathBuf::from("/data/remembered.mlite")),
            ..Config::default()
        };
        let app = App::new(persisted.clone());
        assert_eq!(app.db_path, None);

        let saved = config_to_save(persisted.clone(), &app);
        assert_eq!(saved.last_db_path, persisted.last_db_path);
        assert_eq!(saved.theme, persisted.theme);
    }

    /// The API key and the config keys are masked in logged text
    #[test]
    fn test_startup_secrets_are_registered() {
        let cli = Cli::parse_from(["tuti-tui", "--api-key", "sk-cli-5b77e2"]);
        register_startup_secrets(&cli);
        let persisted = Config {
            mcp_api_key: Some("sk-file-91ac04".to_string()),
            ..Config::default()
        };
        let mut effective = persisted.clone();
        apply_cli_overrides(&mut effective, &cli);
        register_config_secrets(&persisted, &effective);

        assert_eq!(
            logging::sanitize_for_log("denied: sk-cli-5b77e2 / sk-file-91ac04"),
            "denied: [REDACTED] / [REDACTED]"
        );
    }

    #[test]
    fn test_require_terminal() {
        assert_eq!(require_terminal(true), Ok(()));
        let err = require_terminal(false).unwrap_err();
        assert!(err.contains("stdout"));
        assert_eq!(err.lines().count(), 1);
    }

    #[test]
    fn test_log_file_flag() {
        let cli = Cli::parse_from(["tuti-tui", "--log-file", "/tmp/x.log"]);
        assert_eq!(cli.log_file, Some(PathBuf::from("/tmp/x.log")));
        assert_eq!(Cli::parse_from(["tuti-tui"]).log_file, None);
    }
}
