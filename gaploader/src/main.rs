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

        /// Split mode
        #[arg(long, short, default_value = "auto")]
        mode: SplitMode,

        /// Chunk size in characters
        #[arg(long, default_value = "1000")]
        chunk_size: usize,

        /// Overlap size in characters (for text mode only)
        #[arg(long, default_value = "200")]
        overlap: usize,

        /// Generate embeddings after insert
        #[arg(long)]
        embed: bool,

        /// Embedding provider
        #[arg(long, default_value = "fasttext")]
        provider: String,

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

        /// Split mode
        #[arg(long, short, default_value = "auto")]
        mode: SplitMode,

        /// Chunk size in characters
        #[arg(long, default_value = "1000")]
        chunk_size: usize,

        /// Overlap size in characters
        #[arg(long, default_value = "200")]
        overlap: usize,

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
        } => commands::preview::run(file, mode, chunk_size, overlap, limit),

        Commands::List { collection, source } => {
            commands::list::run(&config, collection, source).await
        }
    }
}
