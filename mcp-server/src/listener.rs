//! Listener configuration for multi-interface support
//!
//! Provides:
//! - ListenerConfig for defining multiple listen addresses
//! - Stored in `_system.listeners` collection
//! - Runtime management of listeners

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

/// System collection name for listener configuration
pub const SYSTEM_LISTENERS_COLLECTION: &str = "_system.listeners";

// ============================================================================
// ListenerConfig - Configuration for a single listener
// ============================================================================

/// Configuration for a single HTTP/HTTPS listener
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerConfig {
    /// Unique identifier for this listener
    #[serde(rename = "_id")]
    pub id: String,
    /// Bind address (e.g., "0.0.0.0:8080", "192.168.1.100:443")
    pub bind: String,
    /// Whether TLS is enabled for this listener
    #[serde(default)]
    pub tls: bool,
    /// Path to TLS certificate file (required if tls=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_path: Option<String>,
    /// Path to TLS private key file (required if tls=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    /// Whether this listener is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional description for this listener
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl ListenerConfig {
    /// Create a new HTTP listener
    pub fn new_http(id: impl Into<String>, bind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            bind: bind.into(),
            tls: false,
            cert_path: None,
            key_path: None,
            enabled: true,
            description: None,
        }
    }

    /// Create a new HTTPS listener
    pub fn new_https(
        id: impl Into<String>,
        bind: impl Into<String>,
        cert_path: impl Into<String>,
        key_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            bind: bind.into(),
            tls: true,
            cert_path: Some(cert_path.into()),
            key_path: Some(key_path.into()),
            enabled: true,
            description: None,
        }
    }

    /// Validate the listener configuration
    pub fn validate(&self) -> Result<()> {
        // Check bind address format
        if self.bind.is_empty() {
            return Err(McpError::InvalidParams(
                "Listener bind address cannot be empty".to_string(),
            ));
        }

        // Validate bind address has port
        if !self.bind.contains(':') {
            return Err(McpError::InvalidParams(format!(
                "Listener bind address must include port: {}",
                self.bind
            )));
        }

        // Check TLS configuration
        if self.tls {
            if self.cert_path.is_none() {
                return Err(McpError::InvalidParams(
                    "TLS enabled but cert_path not specified".to_string(),
                ));
            }
            if self.key_path.is_none() {
                return Err(McpError::InvalidParams(
                    "TLS enabled but key_path not specified".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Parse bind address into host and port
    pub fn parse_bind(&self) -> Result<(String, u16)> {
        let parts: Vec<&str> = self.bind.rsplitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(McpError::InvalidParams(format!(
                "Invalid bind address format: {}",
                self.bind
            )));
        }

        let port: u16 = parts[0].parse().map_err(|_| {
            McpError::InvalidParams(format!("Invalid port in bind address: {}", self.bind))
        })?;

        let host = parts[1].to_string();
        Ok((host, port))
    }
}

// ============================================================================
// ListenerManager - Manage listener configurations in database
// ============================================================================

/// Manager for listener configurations stored in `_system.listeners`
pub struct ListenerManager {
    adapter: Arc<IronBaseAdapter>,
}

impl ListenerManager {
    /// Create a new listener manager
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        Self { adapter }
    }

    /// Get all listener configurations
    pub fn list(&self) -> Result<Vec<ListenerConfig>> {
        // Handle missing collection gracefully (fresh database)
        let result = match self.adapter.find(
            SYSTEM_LISTENERS_COLLECTION,
            json!({}),
            FindOptions::default(),
        ) {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()), // Collection doesn't exist yet
        };

        let mut listeners = Vec::new();
        for doc in result.documents {
            if let Ok(listener) = serde_json::from_value::<ListenerConfig>(doc) {
                listeners.push(listener);
            }
        }

        Ok(listeners)
    }

    /// Get enabled listener configurations only
    pub fn list_enabled(&self) -> Result<Vec<ListenerConfig>> {
        // Handle missing collection gracefully (fresh database)
        let result = match self.adapter.find(
            SYSTEM_LISTENERS_COLLECTION,
            json!({ "enabled": true }),
            FindOptions::default(),
        ) {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()), // Collection doesn't exist yet
        };

        let mut listeners = Vec::new();
        for doc in result.documents {
            if let Ok(listener) = serde_json::from_value::<ListenerConfig>(doc) {
                listeners.push(listener);
            }
        }

        Ok(listeners)
    }

    /// Get a specific listener by ID
    pub fn get(&self, id: &str) -> Result<Option<ListenerConfig>> {
        // Handle missing collection gracefully (fresh database)
        let result = match self.adapter.find(
            SYSTEM_LISTENERS_COLLECTION,
            json!({ "_id": id }),
            FindOptions {
                limit: Some(1),
                ..Default::default()
            },
        ) {
            Ok(r) => r,
            Err(_) => return Ok(None), // Collection doesn't exist yet
        };

        if let Some(doc) = result.documents.into_iter().next() {
            let listener = serde_json::from_value::<ListenerConfig>(doc)?;
            Ok(Some(listener))
        } else {
            Ok(None)
        }
    }

    /// Add or update a listener configuration
    pub fn set(&self, config: &ListenerConfig) -> Result<()> {
        // Validate configuration
        config.validate()?;

        let doc = serde_json::to_value(config)?;

        // Check if listener exists (handle case where collection doesn't exist yet)
        if self.get(&config.id).unwrap_or(None).is_some() {
            // Update existing
            self.adapter.update_one(
                SYSTEM_LISTENERS_COLLECTION,
                json!({ "_id": config.id }),
                json!({ "$set": doc }),
            )?;
        } else {
            // Insert new (this will create the collection if it doesn't exist)
            self.adapter.insert_one(SYSTEM_LISTENERS_COLLECTION, doc)?;
        }

        Ok(())
    }

    /// Delete a listener configuration
    pub fn delete(&self, id: &str) -> Result<bool> {
        // Handle missing collection gracefully
        let count = match self
            .adapter
            .delete_one(SYSTEM_LISTENERS_COLLECTION, json!({ "_id": id }))
        {
            Ok(c) => c,
            Err(_) => return Ok(false), // Collection doesn't exist
        };
        Ok(count > 0)
    }

    /// Enable a listener
    pub fn enable(&self, id: &str) -> Result<bool> {
        // Handle missing collection gracefully
        let result = match self.adapter.update_one(
            SYSTEM_LISTENERS_COLLECTION,
            json!({ "_id": id }),
            json!({ "$set": { "enabled": true } }),
        ) {
            Ok(r) => r,
            Err(_) => return Ok(false), // Collection doesn't exist
        };
        Ok(result.matched_count > 0)
    }

    /// Disable a listener
    pub fn disable(&self, id: &str) -> Result<bool> {
        // Handle missing collection gracefully
        let result = match self.adapter.update_one(
            SYSTEM_LISTENERS_COLLECTION,
            json!({ "_id": id }),
            json!({ "$set": { "enabled": false } }),
        ) {
            Ok(r) => r,
            Err(_) => return Ok(false), // Collection doesn't exist
        };
        Ok(result.matched_count > 0)
    }

    /// Initialize default listener if none exist
    pub fn init_default(&self, host: &str, port: u16, tls: bool) -> Result<()> {
        // Check if any listeners exist
        let existing = self.list()?;
        if !existing.is_empty() {
            return Ok(());
        }

        // Create default listener based on current config
        let default = ListenerConfig {
            id: "default".to_string(),
            bind: format!("{}:{}", host, port),
            tls,
            cert_path: None,
            key_path: None,
            enabled: true,
            description: Some("Default listener (created from startup config)".to_string()),
        };

        self.set(&default)?;
        tracing::info!("Initialized default listener: {}", default.bind);

        Ok(())
    }
}

// ============================================================================
// Tool helpers
// ============================================================================

/// Get the required permission for listener operations
pub fn listener_requires_admin() -> bool {
    // All listener operations require admin (localhost only by default ACL)
    true
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listener_config_http() {
        let config = ListenerConfig::new_http("test", "0.0.0.0:8080");
        assert_eq!(config.id, "test");
        assert_eq!(config.bind, "0.0.0.0:8080");
        assert!(!config.tls);
        assert!(config.enabled);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_listener_config_https() {
        let config =
            ListenerConfig::new_https("secure", "0.0.0.0:443", "/path/cert.pem", "/path/key.pem");
        assert!(config.tls);
        assert_eq!(config.cert_path, Some("/path/cert.pem".to_string()));
        assert_eq!(config.key_path, Some("/path/key.pem".to_string()));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_listener_config_validate_empty_bind() {
        let config = ListenerConfig {
            id: "test".to_string(),
            bind: "".to_string(),
            tls: false,
            cert_path: None,
            key_path: None,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_listener_config_validate_no_port() {
        let config = ListenerConfig {
            id: "test".to_string(),
            bind: "0.0.0.0".to_string(),
            tls: false,
            cert_path: None,
            key_path: None,
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_listener_config_validate_tls_no_cert() {
        let config = ListenerConfig {
            id: "test".to_string(),
            bind: "0.0.0.0:443".to_string(),
            tls: true,
            cert_path: None,
            key_path: Some("/path/key.pem".to_string()),
            enabled: true,
            description: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_listener_config_parse_bind() {
        let config = ListenerConfig::new_http("test", "192.168.1.100:8080");
        let (host, port) = config.parse_bind().unwrap();
        assert_eq!(host, "192.168.1.100");
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_listener_config_parse_bind_ipv6() {
        let config = ListenerConfig::new_http("test", "[::]:8080");
        let (host, port) = config.parse_bind().unwrap();
        assert_eq!(host, "[::]");
        assert_eq!(port, 8080);
    }
}
