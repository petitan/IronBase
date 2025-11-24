// Security and authentication for MCP server

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Authentication manager
pub struct AuthManager {
    api_keys: Arc<RwLock<HashMap<String, ApiKey>>>,
    rate_limiters: Arc<RwLock<HashMap<String, RateLimiter>>>,
}

impl AuthManager {
    pub fn new() -> Self {
        Self {
            api_keys: Arc::new(RwLock::new(HashMap::new())),
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load API keys from configuration
    pub fn load_api_keys(&self, keys: Vec<ApiKey>) {
        let mut api_keys = self.api_keys.write().unwrap();
        for key in keys {
            api_keys.insert(key.key.clone(), key);
        }
    }

    /// Authenticate a request
    pub fn authenticate(&self, api_key: &str) -> Result<ApiKey, AuthError> {
        let api_keys = self.api_keys.read().unwrap();
        api_keys
            .get(api_key)
            .cloned()
            .ok_or(AuthError::InvalidApiKey)
    }

    /// Check if command is allowed for this API key
    pub fn authorize(&self, api_key: &ApiKey, command: &str) -> Result<(), AuthError> {
        if let Some(ref whitelist) = api_key.allowed_commands {
            if !whitelist.contains(command) {
                return Err(AuthError::CommandNotAllowed {
                    command: command.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Check if document access is allowed
    pub fn check_document_access(
        &self,
        api_key: &ApiKey,
        document_id: &str,
    ) -> Result<(), AuthError> {
        if let Some(ref allowed) = api_key.allowed_documents {
            if !allowed.contains(document_id) && !allowed.contains("*") {
                return Err(AuthError::DocumentAccessDenied {
                    document_id: document_id.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Check rate limit
    pub fn check_rate_limit(&self, api_key: &str) -> Result<(), AuthError> {
        let mut rate_limiters = self.rate_limiters.write().unwrap();
        let limiter = rate_limiters
            .entry(api_key.to_string())
            .or_insert_with(RateLimiter::new);

        if !limiter.allow() {
            return Err(AuthError::RateLimitExceeded {
                retry_after: limiter.retry_after(),
            });
        }

        Ok(())
    }

    /// Check write operation rate limit (stricter)
    pub fn check_write_rate_limit(&self, api_key: &str) -> Result<(), AuthError> {
        let mut rate_limiters = self.rate_limiters.write().unwrap();
        let limiter = rate_limiters
            .entry(format!("{}_write", api_key))
            .or_insert_with(RateLimiter::new_write_limiter);

        if !limiter.allow() {
            return Err(AuthError::RateLimitExceeded {
                retry_after: limiter.retry_after(),
            });
        }

        Ok(())
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// API key configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub key: String,
    pub name: String,
    pub allowed_commands: Option<HashSet<String>>,
    pub allowed_documents: Option<HashSet<String>>,
    pub rate_limit: Option<RateLimitConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub writes_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 100,
            writes_per_minute: 10,
        }
    }
}

/// Rate limiter using token bucket algorithm
pub struct RateLimiter {
    capacity: u32,
    tokens: f64,
    refill_rate: f64,      // Tokens per second
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            capacity: 100,
            tokens: 100.0,
            refill_rate: 100.0 / 60.0, // 100 per minute
            last_refill: Instant::now(),
        }
    }

    pub fn new_write_limiter() -> Self {
        Self {
            capacity: 10,
            tokens: 10.0,
            refill_rate: 10.0 / 60.0, // 10 per minute
            last_refill: Instant::now(),
        }
    }

    pub fn new_with_config(requests_per_minute: u32) -> Self {
        Self {
            capacity: requests_per_minute,
            tokens: requests_per_minute as f64,
            refill_rate: requests_per_minute as f64 / 60.0,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.refill_rate;

        self.tokens = (self.tokens + new_tokens).min(self.capacity as f64);
        self.last_refill = now;
    }

    pub fn allow(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn retry_after(&self) -> Duration {
        let tokens_needed = 1.0 - self.tokens;
        let seconds = tokens_needed / self.refill_rate;
        Duration::from_secs_f64(seconds.max(1.0))
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Authentication errors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthError {
    InvalidApiKey,
    CommandNotAllowed { command: String },
    DocumentAccessDenied { document_id: String },
    RateLimitExceeded { retry_after: Duration },
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::InvalidApiKey => write!(f, "Invalid API key"),
            AuthError::CommandNotAllowed { command } => {
                write!(f, "Command not allowed: {}", command)
            }
            AuthError::DocumentAccessDenied { document_id } => {
                write!(f, "Document access denied: {}", document_id)
            }
            AuthError::RateLimitExceeded { retry_after } => {
                write!(f, "Rate limit exceeded, retry after {:?}", retry_after)
            }
        }
    }
}

impl std::error::Error for AuthError {}

/// Command whitelist - only these commands are allowed for AI agents
pub fn default_whitelist() -> HashSet<String> {
    [
        "mcp_docjl_list_documents",
        "mcp_docjl_get_document",
        "mcp_docjl_insert_block",
        "mcp_docjl_update_block",
        "mcp_docjl_move_block",
        "mcp_docjl_delete_block",
        "mcp_docjl_list_headings",
        "mcp_docjl_search_blocks",
        "mcp_docjl_search_content",
        "mcp_docjl_validate_references",
        "mcp_docjl_validate_schema",
        "mcp_docjl_get_audit_log",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Read-only commands (no rate limit for writes)
pub fn read_only_commands() -> HashSet<String> {
    [
        "mcp_docjl_list_documents",
        "mcp_docjl_get_document",
        "mcp_docjl_list_headings",
        "mcp_docjl_search_blocks",
        "mcp_docjl_search_content",
        "mcp_docjl_validate_references",
        "mcp_docjl_validate_schema",
        "mcp_docjl_get_audit_log",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Dangerous commands that require confirmation
pub fn dangerous_commands() -> HashSet<String> {
    ["mcp_docjl_delete_block"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_manager() {
        let auth = AuthManager::new();

        let api_key = ApiKey {
            key: "test_key_123".to_string(),
            name: "Test Key".to_string(),
            allowed_commands: Some(default_whitelist()),
            allowed_documents: Some(["doc_1".to_string()].iter().cloned().collect()),
            rate_limit: Some(RateLimitConfig::default()),
        };

        auth.load_api_keys(vec![api_key.clone()]);

        let authenticated = auth.authenticate("test_key_123");
        assert!(authenticated.is_ok());

        let invalid = auth.authenticate("invalid_key");
        assert!(invalid.is_err());
    }

    #[test]
    fn test_authorization() {
        let auth = AuthManager::new();

        let api_key = ApiKey {
            key: "test_key".to_string(),
            name: "Test".to_string(),
            allowed_commands: Some(["mcp_docjl_get_document".to_string()].iter().cloned().collect()),
            allowed_documents: None,
            rate_limit: None,
        };

        assert!(auth.authorize(&api_key, "mcp_docjl_get_document").is_ok());
        assert!(auth.authorize(&api_key, "mcp_docjl_delete_block").is_err());
    }

    #[test]
    fn test_document_access() {
        let auth = AuthManager::new();

        let api_key = ApiKey {
            key: "test_key".to_string(),
            name: "Test".to_string(),
            allowed_commands: None,
            allowed_documents: Some(["doc_1".to_string(), "doc_2".to_string()].iter().cloned().collect()),
            rate_limit: None,
        };

        assert!(auth.check_document_access(&api_key, "doc_1").is_ok());
        assert!(auth.check_document_access(&api_key, "doc_2").is_ok());
        assert!(auth.check_document_access(&api_key, "doc_3").is_err());
    }

    #[test]
    fn test_wildcard_document_access() {
        let auth = AuthManager::new();

        let api_key = ApiKey {
            key: "test_key".to_string(),
            name: "Test".to_string(),
            allowed_commands: None,
            allowed_documents: Some(["*".to_string()].iter().cloned().collect()),
            rate_limit: None,
        };

        assert!(auth.check_document_access(&api_key, "any_doc").is_ok());
    }

    #[test]
    fn test_rate_limiter() {
        let mut limiter = RateLimiter {
            capacity: 2,
            tokens: 2.0,
            refill_rate: 1.0,
            last_refill: Instant::now(),
        };

        assert!(limiter.allow());
        assert!(limiter.allow());
        assert!(!limiter.allow()); // Exceeded

        std::thread::sleep(Duration::from_millis(1100));
        assert!(limiter.allow()); // Refilled
    }
}
