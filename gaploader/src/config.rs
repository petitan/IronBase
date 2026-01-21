//! Configuration loading from TOML file and environment variables

use crate::error::{GaploaderError, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// Bridge configuration
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BridgeConfig {
    /// Path to ironbase-bridge executable
    #[serde(default)]
    pub path: Option<String>,

    /// MCP server URL
    #[serde(default = "default_server_url")]
    pub server_url: String,

    /// API key for authentication
    #[serde(default)]
    pub api_key: Option<String>,

    /// Accept self-signed certificates
    #[serde(default)]
    pub insecure: bool,
}

fn default_server_url() -> String {
    "http://127.0.0.1:8080/mcp".to_string()
}

/// Chunking configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ChunkingConfig {
    /// Default split mode: "auto", "markdown", "text"
    #[serde(default = "default_mode")]
    pub default_mode: String,

    /// Chunk size in characters
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Overlap for text mode
    #[serde(default = "default_overlap")]
    pub overlap: usize,
}

fn default_mode() -> String {
    "auto".to_string()
}

fn default_chunk_size() -> usize {
    1000
}

fn default_overlap() -> usize {
    200
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            default_mode: default_mode(),
            chunk_size: default_chunk_size(),
            overlap: default_overlap(),
        }
    }
}

/// Embedding configuration
#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingConfig {
    /// Enable embedding by default
    #[serde(default)]
    pub enabled: bool,

    /// Default embedding provider
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Batch size for embed_batch calls
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_provider() -> String {
    "fasttext".to_string()
}

fn default_batch_size() -> usize {
    50
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            batch_size: default_batch_size(),
        }
    }
}

/// Collection naming configuration
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionConfig {
    /// Prefix for all collection names
    #[serde(default)]
    pub prefix: String,

    /// Suffix for all collection names
    #[serde(default)]
    pub suffix: String,

    /// Convert filenames to lowercase
    #[serde(default = "default_true")]
    pub lowercase: bool,
}

fn default_true() -> bool {
    true
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            suffix: String::new(),
            lowercase: true,
        }
    }
}

/// Main configuration structure
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub bridge: BridgeConfig,

    #[serde(default)]
    pub chunking: ChunkingConfig,

    #[serde(default)]
    pub embedding: EmbeddingConfig,

    #[serde(default)]
    pub collection: CollectionConfig,
}

impl Config {
    /// Load configuration from file, with environment variable overrides
    pub fn load(path: Option<&PathBuf>) -> Result<Self> {
        let mut config = if let Some(path) = path {
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                toml::from_str(&content)?
            } else {
                Config::default()
            }
        } else {
            // Try default locations
            let default_path = Self::default_config_path();
            if default_path.exists() {
                let content = std::fs::read_to_string(&default_path)?;
                toml::from_str(&content)?
            } else {
                Config::default()
            }
        };

        // Apply environment variable overrides
        config.apply_env_overrides();

        Ok(config)
    }

    /// Get default config file path
    pub fn default_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gaploader")
            .join("config.toml")
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(&mut self) {
        // Bridge config
        if let Ok(path) = std::env::var("GAPLOADER_BRIDGE_PATH") {
            self.bridge.path = Some(path);
        }
        if let Ok(url) = std::env::var("MCP_SERVER_URL") {
            self.bridge.server_url = url;
        }
        if let Ok(key) = std::env::var("IRONBASE_API_KEY") {
            self.bridge.api_key = Some(key);
        }
        if let Ok(insecure) = std::env::var("MCP_INSECURE") {
            self.bridge.insecure = matches!(
                insecure.to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
    }

    /// Find bridge executable path
    pub fn find_bridge_path(&self) -> Result<PathBuf> {
        // 1. Config path
        if let Some(ref path) = self.bridge.path {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }

        // 2. Search in PATH
        if let Ok(path) = which::which("ironbase-bridge") {
            return Ok(path);
        }

        // 3. Platform-specific locations
        #[cfg(target_os = "linux")]
        {
            let p = PathBuf::from("/usr/local/bin/ironbase-bridge");
            if p.exists() {
                return Ok(p);
            }
        }

        #[cfg(target_os = "macos")]
        {
            let p = PathBuf::from("/usr/local/bin/ironbase-bridge");
            if p.exists() {
                return Ok(p);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                let p = PathBuf::from(local_app_data)
                    .join("IronBase")
                    .join("bin")
                    .join("ironbase-bridge.exe");
                if p.exists() {
                    return Ok(p);
                }
            }
        }

        Err(GaploaderError::BridgeNotFound(
            "ironbase-bridge not found. Install it or set GAPLOADER_BRIDGE_PATH".to_string(),
        ))
    }

    /// Generate collection name from filename
    pub fn collection_name(&self, filename: &str) -> String {
        let stem = std::path::Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);

        let mut name = format!("{}{}{}", self.collection.prefix, stem, self.collection.suffix);

        if self.collection.lowercase {
            name = name.to_lowercase();
        }

        // Sanitize: replace spaces and special chars with underscores
        name.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
}
