//! Configuration management

use crate::theme::ThemeName;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// MCP Transport mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    /// HTTP transport (connect to external MCP server)
    #[default]
    Http,
    /// Stdio transport (spawn MCP server as child process)
    Stdio,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Current theme
    #[serde(default)]
    pub theme: ThemeName,

    /// Last opened database path
    pub last_db_path: Option<PathBuf>,

    /// Claude API key (for natural language queries)
    pub anthropic_api_key: Option<String>,

    /// Recipes directory
    pub recipes_dir: Option<PathBuf>,

    // === MCP Settings ===
    /// Transport mode (http or stdio)
    #[serde(default)]
    pub transport: TransportMode,

    /// MCP server HTTP URL (for Http transport)
    #[serde(default = "default_mcp_url")]
    pub mcp_url: String,

    /// MCP server binary path (for Stdio transport)
    pub mcp_server_path: Option<PathBuf>,
}

fn default_mcp_url() -> String {
    "http://127.0.0.1:8080".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: ThemeName::Claude,
            last_db_path: None,
            anthropic_api_key: None,
            recipes_dir: None,
            transport: TransportMode::Http,
            mcp_url: default_mcp_url(),
            mcp_server_path: None,
        }
    }
}

impl Config {
    /// Get config file path
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("ironbase-tui").join("config.toml"))
    }

    /// Get default recipes directory
    pub fn default_recipes_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("ironbase-tui").join("recipes"))
    }

    /// Get default MCP server path (relative to workspace)
    pub fn default_mcp_server_path() -> PathBuf {
        // Try to find the MCP server binary
        let candidates = [
            PathBuf::from("./mcp-server/target/release/mcp-ironbase-server"),
            PathBuf::from("../mcp-server/target/release/mcp-ironbase-server"),
            PathBuf::from("/usr/local/bin/mcp-ironbase-server"),
        ];

        for path in candidates {
            if path.exists() {
                return path;
            }
        }

        // Fallback to default path
        PathBuf::from("./mcp-server/target/release/mcp-ironbase-server")
    }

    /// Load config from file
    pub fn load() -> Self {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return Self::default(),
        };

        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save config to file
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("No config directory"))?;

        // Create parent directory
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Get API key from config or environment
    pub fn get_api_key(&self) -> Option<String> {
        self.anthropic_api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }

    /// Get recipes directory (creates if needed)
    pub fn get_recipes_dir(&self) -> Option<PathBuf> {
        let dir = self
            .recipes_dir
            .clone()
            .or_else(Self::default_recipes_dir)?;

        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }

        Some(dir)
    }

    /// Get MCP server path (from config or default)
    pub fn get_mcp_server_path(&self) -> PathBuf {
        self.mcp_server_path
            .clone()
            .unwrap_or_else(Self::default_mcp_server_path)
    }
}
