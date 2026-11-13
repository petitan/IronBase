//! Configuration management

use crate::theme::ThemeName;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    /// MCP server API key (for Http transport authentication)
    pub mcp_api_key: Option<String>,

    /// Accept self-signed/invalid TLS certificates (for HTTPS)
    #[serde(default)]
    pub mcp_insecure: bool,
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
            mcp_api_key: None,
            mcp_insecure: false,
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

    /// Load config from file (defaults on any error)
    pub fn load() -> Self {
        Self::try_load().unwrap_or_default()
    }

    /// Load config from file. A missing file is the default config; an
    /// unreadable or invalid file is an error, and the caller must not save
    /// over it (that would replace the user's file with defaults).
    pub fn try_load() -> anyhow::Result<Self> {
        match Self::config_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }

    fn load_from(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?;
        toml::from_str(&content).map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))
    }

    /// Save config to file
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path().ok_or_else(|| anyhow::anyhow!("No config directory"))?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        // Create parent directory
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;

        // The file can hold API keys: owner-only from the start, and an
        // existing file with wider permissions is narrowed too
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.write_all(content.as_bytes())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, content)?;
        }
        Ok(())
    }

    /// Get Anthropic API key from config or environment
    pub fn get_api_key(&self) -> Option<String> {
        self.anthropic_api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
    }

    /// Get MCP API key from config or environment
    pub fn get_mcp_api_key(&self) -> Option<String> {
        self.mcp_api_key
            .clone()
            .or_else(|| std::env::var("IRONBASE_API_KEY").ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tuti-tui-config-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn test_missing_config_file_is_default() {
        let path = temp_config_path("missing");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.mcp_url, default_mcp_url());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A broken file must be an error, not silently the default config
    /// (the exit-save would then overwrite the user's file)
    #[test]
    fn test_invalid_config_file_is_an_error() {
        let path = temp_config_path("invalid");
        std::fs::write(&path, "theme = [this is not toml").unwrap();
        let err = Config::load_from(&path).unwrap_err();
        assert!(err.to_string().contains("config.toml"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_save_and_load_round_trip() {
        let path = temp_config_path("roundtrip");
        let config = Config {
            mcp_url: "https://db.example:9000/mcp".to_string(),
            last_db_path: Some(PathBuf::from("/data/x.mlite")),
            ..Config::default()
        };
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.mcp_url, config.mcp_url);
        assert_eq!(loaded.last_db_path, config.last_db_path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_config_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_config_path("mode");

        // New file
        Config::default().save_to(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Existing world-readable file is narrowed
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        Config::default().save_to(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
