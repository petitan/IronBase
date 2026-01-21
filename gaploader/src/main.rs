//! Gaploader - Load text/markdown files into IronBase via MCP

mod bridge;
mod chunk;
mod commands;
mod config;
mod error;
mod loader;
mod mcp_client;
mod splitter;

use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use error::Result;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "gaploader")]
#[command(version, about = "Load text/markdown files into IronBase via MCP")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Config file path
    #[arg(long, short, env = "GAPLOADER_CONFIG", global = true)]
    config: Option<PathBuf>,

    /// Bridge executable path (overrides config)
    #[arg(long, env = "GAPLOADER_BRIDGE_PATH", global = true)]
    bridge: Option<PathBuf>,

    /// MCP server URL (passed to bridge)
    #[arg(long, env = "MCP_SERVER_URL", global = true)]
    server: Option<String>,

    /// API key for MCP server
    #[arg(long, short = 'k', env = "IRONBASE_API_KEY", global = true)]
    api_key: Option<String>,

    /// Accept self-signed TLS certificates
    #[arg(long, env = "MCP_INSECURE", global = true)]
    insecure: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Load a file into IronBase
    #[command(alias = "l")]
    Load {
        /// Input file path (markdown or plain text)
        file: PathBuf,

        /// Target collection name (default: filename without extension)
        #[arg(long)]
        collection: Option<String>,

        /// Split mode (default: from config or "auto")
        #[arg(long, short)]
        mode: Option<SplitMode>,

        /// Chunk size in characters (default: from config or 1000)
        #[arg(long)]
        chunk_size: Option<usize>,

        /// Overlap size in characters for text mode (default: from config or 200)
        #[arg(long)]
        overlap: Option<usize>,

        /// Generate embeddings after insert (default: from config)
        #[arg(long)]
        embed: bool,

        /// Embedding provider (default: from config or "fasttext")
        #[arg(long)]
        provider: Option<String>,

        /// Dry run - show chunks without inserting
        #[arg(long)]
        dry_run: bool,

        /// Clear collection before loading
        #[arg(long)]
        clear: bool,
    },

    /// Preview chunks without loading
    #[command(alias = "p")]
    Preview {
        /// Input file path
        file: PathBuf,

        /// Split mode (default: from config or "auto")
        #[arg(long, short)]
        mode: Option<SplitMode>,

        /// Chunk size in characters (default: from config or 1000)
        #[arg(long)]
        chunk_size: Option<usize>,

        /// Overlap size in characters (default: from config or 200)
        #[arg(long)]
        overlap: Option<usize>,

        /// Show first N chunks only
        #[arg(long, short, default_value = "5")]
        limit: usize,
    },

    /// List loaded files in a collection
    #[command(alias = "ls")]
    List {
        /// Collection name (optional, lists all if not specified)
        collection: Option<String>,

        /// Filter by source file pattern
        #[arg(long)]
        source: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq)]
pub enum SplitMode {
    /// Auto-detect from file extension (.md → markdown, else → text)
    Auto,
    /// Markdown mode: split at headings, no overlap
    Markdown,
    /// Text mode: fixed size chunks with overlap
    Text,
}

impl SplitMode {
    /// Detect mode from file extension
    pub fn detect(path: &std::path::Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("md") | Some("markdown") => SplitMode::Markdown,
            _ => SplitMode::Text,
        }
    }

    /// Resolve auto mode to concrete mode
    pub fn resolve(self, path: &std::path::Path) -> Self {
        match self {
            SplitMode::Auto => Self::detect(path),
            other => other,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(e) = run(cli).await {
        eprintln!("\x1b[31mError:\x1b[0m {}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

async fn run(cli: Cli) -> Result<()> {
    // Load config
    let mut config = Config::load(cli.config.as_ref())?;

    // Apply CLI overrides
    if let Some(ref bridge) = cli.bridge {
        config.bridge.path = Some(bridge.to_string_lossy().to_string());
    }
    if let Some(ref server) = cli.server {
        config.bridge.server_url = server.clone();
    }
    if let Some(ref key) = cli.api_key {
        config.bridge.api_key = Some(key.clone());
    }
    if cli.insecure {
        config.bridge.insecure = true;
    }

    match cli.command {
        Commands::Load {
            file,
            collection,
            mode,
            chunk_size,
            overlap,
            embed,
            provider,
            dry_run,
            clear,
        } => {
            // Apply config defaults
            let mode = mode.unwrap_or_else(|| parse_mode(&config.chunking.default_mode));
            let chunk_size = chunk_size.unwrap_or(config.chunking.chunk_size);
            let overlap = overlap.unwrap_or(config.chunking.overlap);
            let embed = embed || config.embedding.enabled;
            let provider = provider.unwrap_or_else(|| config.embedding.provider.clone());

            commands::load::run(
                &config,
                file,
                collection,
                mode,
                chunk_size,
                overlap,
                embed,
                provider,
                dry_run,
                clear,
            )
            .await
        }

        Commands::Preview {
            file,
            mode,
            chunk_size,
            overlap,
            limit,
        } => {
            // Apply config defaults
            let mode = mode.unwrap_or_else(|| parse_mode(&config.chunking.default_mode));
            let chunk_size = chunk_size.unwrap_or(config.chunking.chunk_size);
            let overlap = overlap.unwrap_or(config.chunking.overlap);

            commands::preview::run(file, mode, chunk_size, overlap, limit)
        }

        Commands::List { collection, source } => {
            commands::list::run(&config, collection, source).await
        }
    }
}

/// Parse mode string from config
fn parse_mode(mode_str: &str) -> SplitMode {
    match mode_str.to_lowercase().as_str() {
        "markdown" | "md" => SplitMode::Markdown,
        "text" | "txt" => SplitMode::Text,
        _ => SplitMode::Auto,
    }
}
