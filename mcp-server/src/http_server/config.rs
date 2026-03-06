//! Configuration loading and TOML parsing for HTTP server

use super::size::parse_size;
use std::path::PathBuf;

/// Default max body size: 10 MB (protects against DoS via large requests)
/// Can be overridden via config.toml or IRONBASE_MAX_BODY_SIZE env var
const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Default tool timeout: 25 seconds
/// SAFETY: Must be SHORTER than Claude Desktop's client timeout (~30s)
/// Otherwise: client times out → sends new request → server still working on old one → backlog
/// If a query takes longer than 25s, it MUST be optimized with indexes/limits
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 25;

/// Configuration for HTTP server
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub max_body_size: usize,
    /// If true, API key is required for all tool calls
    pub require_api_key: bool,
    /// Cache TTL for API keys in seconds
    pub api_key_cache_ttl: u64,
    /// If true, use HTTPS instead of HTTP
    pub tls_enabled: bool,
    /// Path to TLS certificate file
    pub tls_cert_file: Option<String>,
    /// Path to TLS key file
    pub tls_key_file: Option<String>,
    /// Timeout for long-running tool operations in seconds (default: 300)
    pub tool_timeout_secs: u64,
    /// If true, use synchronous (fsync) logging for crash debugging
    pub sync_logging: bool,
    /// Log level for ironbase-core internal logging (error, warn, info, debug, trace)
    pub core_log_level: Option<String>,
    /// Path to FastText embedding model for RAG/semantic search
    pub fasttext_model: Option<String>,
    /// Auto-compaction configuration
    pub auto_compact: crate::compaction::AutoCompactConfig,
    /// Embedding provider configuration (takes priority over fasttext_model)
    pub embedding: Option<EmbeddingTomlConfig>,
}

/// Load configuration from environment or config file
/// Priority: CLI args (via env vars) > config file next to executable > defaults
pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = std::env::var("MCP_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    // Try to find config file:
    // 1. Specified path (absolute or relative to cwd)
    // 2. Next to the executable
    let config_path = if std::path::Path::new(&config_path).exists() {
        std::path::PathBuf::from(&config_path)
    } else if let Ok(exe_path) = std::env::current_exe() {
        let exe_dir_config = exe_path.parent().map(|p| p.join("config.toml"));
        if let Some(ref path) = exe_dir_config {
            if path.exists() {
                path.clone()
            } else {
                std::path::PathBuf::from(&config_path)
            }
        } else {
            std::path::PathBuf::from(&config_path)
        }
    } else {
        std::path::PathBuf::from(&config_path)
    };

    let mut config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        // Normalize Windows CRLF to LF for TOML parsing
        let content = content.replace("\r\n", "\n");
        let toml_config: TomlConfig =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;

        // Parse max_body_size if provided, otherwise use default
        let max_body_size = match &toml_config.server.max_body_size {
            Some(size_str) => parse_size(size_str)
                .map_err(|e| format!("Invalid max_body_size '{}': {}", size_str, e))?,
            None => DEFAULT_MAX_BODY_SIZE,
        };

        Config {
            host: toml_config.server.host,
            port: toml_config.server.port,
            database_path: PathBuf::from(toml_config.database.path),
            max_body_size,
            require_api_key: toml_config.security.require_api_key,
            api_key_cache_ttl: toml_config.security.api_key_cache_ttl,
            tls_enabled: toml_config.tls.enabled,
            tls_cert_file: toml_config.tls.cert_file,
            tls_key_file: toml_config.tls.key_file,
            tool_timeout_secs: toml_config.server.tool_timeout_secs,
            sync_logging: toml_config.logging.sync,
            core_log_level: toml_config.logging.core_level,
            fasttext_model: toml_config.rag.fasttext_model,
            auto_compact: toml_config.compaction,
            embedding: toml_config.embedding,
        }
    } else {
        // Check for IRONBASE_PATH env var
        let db_path =
            std::env::var("IRONBASE_PATH").unwrap_or_else(|_| "ironbase_data.mlite".to_string());
        Config {
            host: "0.0.0.0".to_string(),
            port: 8080,
            database_path: PathBuf::from(db_path),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            require_api_key: false,
            api_key_cache_ttl: 60,
            tls_enabled: false,
            tls_cert_file: None,
            tls_key_file: None,
            tool_timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
            sync_logging: false,
            core_log_level: None,
            fasttext_model: None,
            auto_compact: crate::compaction::AutoCompactConfig::default(),
            embedding: None,
        }
    };

    // CLI overrides via environment variables (set by main.rs from CLI args)
    if let Ok(port) = std::env::var("MCP_PORT") {
        if let Ok(p) = port.parse::<u16>() {
            config.port = p;
        }
    }
    if let Ok(host) = std::env::var("MCP_HOST") {
        config.host = host;
    }
    if let Ok(db_path) = std::env::var("IRONBASE_PATH") {
        config.database_path = PathBuf::from(db_path);
    }

    Ok(config)
}

// ============================================================================
// TOML Configuration Structs (internal)
// ============================================================================

#[derive(Debug, serde::Deserialize)]
struct TomlConfig {
    server: ServerConfig,
    database: DatabaseConfig,
    #[serde(default)]
    security: SecurityConfig,
    #[serde(default)]
    tls: TlsConfig,
    #[serde(default)]
    logging: LoggingConfig,
    #[serde(default)]
    rag: RagConfig,
    #[serde(default)]
    compaction: crate::compaction::AutoCompactConfig,
    #[serde(default)]
    embedding: Option<EmbeddingTomlConfig>,
}

#[derive(Debug, serde::Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
    /// Max body size in human-readable format: "1GB", "500MB", "10KB"
    #[serde(default)]
    max_body_size: Option<String>,
    /// Timeout for long-running tool operations in seconds (default: 300)
    #[serde(default = "default_tool_timeout")]
    tool_timeout_secs: u64,
}

fn default_tool_timeout() -> u64 {
    DEFAULT_TOOL_TIMEOUT_SECS
}

#[derive(Debug, serde::Deserialize)]
struct DatabaseConfig {
    path: String,
}

#[derive(Debug, serde::Deserialize, Default)]
struct SecurityConfig {
    /// If true, API key is required for all tool calls
    #[serde(default)]
    require_api_key: bool,
    /// Cache TTL for API keys in seconds (default: 60)
    #[serde(default = "default_cache_ttl")]
    api_key_cache_ttl: u64,
}

fn default_cache_ttl() -> u64 {
    60
}

#[derive(Debug, serde::Deserialize, Default)]
struct TlsConfig {
    /// If true, server uses HTTPS instead of HTTP
    #[serde(default)]
    enabled: bool,
    /// Path to TLS certificate file (PEM format)
    #[serde(default)]
    cert_file: Option<String>,
    /// Path to TLS private key file (PEM format)
    #[serde(default)]
    key_file: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct LoggingConfig {
    /// If true, use synchronous (fsync) logging for crash debugging
    /// WARNING: This significantly impacts performance but guarantees log writes before crash
    #[serde(default)]
    sync: bool,
    /// Log level for ironbase-core internal logging (error, warn, info, debug, trace)
    /// Default: warn (production), override with IRONBASE_LOG_LEVEL env var
    #[serde(default)]
    core_level: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct RagConfig {
    /// Path to FastText embedding model (.bin or .ironbase.bin)
    #[serde(default)]
    fasttext_model: Option<String>,
}

/// Embedding provider configuration from [embedding] TOML section
///
/// When present, this section takes priority over [rag].fasttext_model.
/// Supports: "fasttext", "ollama", "vllm", "openai"
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct EmbeddingTomlConfig {
    /// Provider type: "fasttext" | "ollama" | "vllm" | "openai"
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    /// Base URL for HTTP-based providers (e.g., "http://localhost:11434")
    #[serde(default)]
    pub base_url: Option<String>,
    /// Model name (e.g., "bge-m3", "nomic-embed-text")
    #[serde(default)]
    pub model: Option<String>,
    /// Batch size for backfill jobs (default: 32 for HTTP, 100 for FastText)
    #[serde(default)]
    pub batch_size: Option<usize>,
    /// Request timeout in seconds (default: 120)
    #[serde(default = "default_embedding_timeout")]
    pub timeout_secs: u64,
    /// Max retries for transient errors (default: 3)
    #[serde(default = "default_embedding_max_retries")]
    pub max_retries: usize,
    /// Base delay for exponential backoff in ms (default: 500)
    #[serde(default = "default_retry_base_delay")]
    pub retry_base_delay_ms: u64,
    /// API key (for OpenAI/cloud providers)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Path to FastText model (only used when provider = "fasttext")
    #[serde(default)]
    pub model_path: Option<String>,
}

fn default_embedding_provider() -> String {
    "fasttext".to_string()
}

fn default_embedding_timeout() -> u64 {
    120
}

fn default_embedding_max_retries() -> usize {
    3
}

fn default_retry_base_delay() -> u64 {
    500
}
