//! API Key management module
//!
//! Provides API key storage, caching, and validation for the MCP server.
//! API keys are stored in the `_system.api_keys` collection.

use crate::{FindOptions, IronBaseAdapter};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// API key entry stored in _system.api_keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub _id: u64,
    pub key: String,
    pub name: String,
    pub created_at: String,
    pub enabled: bool,
}

/// API key cache with TTL-based refresh
pub struct ApiKeyCache {
    /// Set of enabled API keys
    keys: RwLock<HashSet<String>>,
    /// Last refresh time
    last_refresh: RwLock<Instant>,
    /// Cache TTL
    ttl: Duration,
    /// Whether API key is required
    required: bool,
}

impl ApiKeyCache {
    /// Create a new API key cache
    pub fn new(ttl_secs: u64, required: bool) -> Self {
        Self {
            keys: RwLock::new(HashSet::new()),
            last_refresh: RwLock::new(Instant::now() - Duration::from_secs(ttl_secs + 1)), // Force initial refresh
            ttl: Duration::from_secs(ttl_secs),
            required,
        }
    }

    /// Check if API key validation is required
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// Refresh cache from database if TTL expired (double-checked locking)
    pub fn refresh_if_needed(&self, adapter: &Arc<IronBaseAdapter>) {
        let now = Instant::now();

        // Quick check with read lock
        {
            let last = self.last_refresh.read();
            if now.duration_since(*last) <= self.ttl {
                return;
            }
        }

        // Acquire write lock and double-check (another thread may have refreshed)
        {
            let mut last_refresh = self.last_refresh.write();
            if now.duration_since(*last_refresh) <= self.ttl {
                // Another thread refreshed while we waited for the lock
                return;
            }
            // Mark as refreshed NOW to prevent other threads from also refreshing
            *last_refresh = now;
        }

        // Do the actual refresh (lock released, so other threads won't block)
        self.do_refresh(adapter);
    }

    /// Force refresh cache from database
    pub fn refresh(&self, adapter: &Arc<IronBaseAdapter>) {
        *self.last_refresh.write() = Instant::now();
        self.do_refresh(adapter);
    }

    /// Internal refresh implementation
    fn do_refresh(&self, adapter: &Arc<IronBaseAdapter>) {
        // Ensure the _system.api_keys collection exists
        let _ = adapter.create_system_collection("_system.api_keys");

        // Create index on 'key' field for fast lookup
        let _ = adapter.create_index("_system.api_keys", "key", true);

        // Load all enabled keys
        let options = FindOptions::default();
        match adapter.find(
            "_system.api_keys",
            serde_json::json!({"enabled": true}),
            options,
        ) {
            Ok(result) => {
                let mut keys = self.keys.write();
                keys.clear();
                for doc in result.documents {
                    if let Some(key) = doc.get("key").and_then(|v| v.as_str()) {
                        keys.insert(key.to_string());
                    }
                }
                tracing::debug!("API key cache refreshed: {} keys loaded", keys.len());
            }
            Err(e) => {
                tracing::warn!("Failed to refresh API key cache: {}", e);
            }
        }
    }

    /// Validate an API key (constant-time comparison)
    pub fn validate(&self, key: &str, adapter: &Arc<IronBaseAdapter>) -> bool {
        self.refresh_if_needed(adapter);

        let keys = self.keys.read();

        // Use constant-time comparison to prevent timing attacks
        let mut found = false;
        for cached_key in keys.iter() {
            if constant_time_compare(key.as_bytes(), cached_key.as_bytes()) {
                found = true;
            }
        }
        found
    }

    /// Invalidate cache (force refresh on next validation)
    pub fn invalidate(&self) {
        *self.last_refresh.write() = Instant::now() - self.ttl - Duration::from_secs(1);
    }
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Generate a random API key using OS entropy via RandomState
pub fn generate_api_key() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    // RandomState uses OS entropy for its SipHasher seed
    let random_state = RandomState::new();
    let mut hasher = random_state.build_hasher();
    hasher.write_u128(timestamp);
    hasher.write_usize(std::process::id() as usize);
    let part1 = hasher.finish();

    // Second round with fresh RandomState for more entropy
    let random_state2 = RandomState::new();
    let mut hasher2 = random_state2.build_hasher();
    hasher2.write_u64(part1);
    hasher2.write_u128(timestamp.wrapping_mul(0x517cc1b727220a95));
    let part2 = hasher2.finish();

    format!("sk-{:016x}{:016x}", part1, part2)
}

/// Create a new API key in the database
pub fn create_api_key(
    adapter: &Arc<IronBaseAdapter>,
    name: &str,
    cache: &ApiKeyCache,
) -> Result<ApiKey, String> {
    // Ensure collection exists
    let _ = adapter.create_system_collection("_system.api_keys");

    // Get next ID by finding max _id
    let options = FindOptions {
        sort: Some(vec![("_id".to_string(), -1)]),
        limit: Some(1),
        ..Default::default()
    };

    let next_id = match adapter.find("_system.api_keys", serde_json::json!({}), options) {
        Ok(result) => {
            if let Some(doc) = result.documents.first() {
                doc.get("_id").and_then(|v| v.as_u64()).unwrap_or(0) + 1
            } else {
                1
            }
        }
        Err(_) => 1,
    };

    let key = generate_api_key();
    let now = chrono::Utc::now().to_rfc3339();

    let api_key = ApiKey {
        _id: next_id,
        key: key.clone(),
        name: name.to_string(),
        created_at: now,
        enabled: true,
    };

    // Insert into database as JSON Value
    let doc = serde_json::to_value(&api_key).unwrap();

    adapter
        .insert_one("_system.api_keys", doc)
        .map_err(|e| format!("Failed to create API key: {}", e))?;

    // Invalidate cache to pick up new key
    cache.invalidate();

    Ok(api_key)
}

/// List all API keys (without showing full key values)
pub fn list_api_keys(adapter: &Arc<IronBaseAdapter>) -> Result<Vec<serde_json::Value>, String> {
    let options = FindOptions {
        sort: Some(vec![("_id".to_string(), 1)]),
        ..Default::default()
    };

    match adapter.find("_system.api_keys", serde_json::json!({}), options) {
        Ok(result) => {
            let masked: Vec<serde_json::Value> = result
                .documents
                .into_iter()
                .map(|doc| {
                    let id = doc.get("_id").cloned().unwrap_or(serde_json::Value::Null);
                    let name = doc
                        .get("name")
                        .cloned()
                        .unwrap_or(serde_json::Value::String("".to_string()));
                    let created_at = doc
                        .get("created_at")
                        .cloned()
                        .unwrap_or(serde_json::Value::String("".to_string()));
                    let enabled = doc
                        .get("enabled")
                        .cloned()
                        .unwrap_or(serde_json::Value::Bool(false));
                    let key = doc
                        .get("key")
                        .and_then(|v| v.as_str())
                        .map(|k| {
                            if k.len() > 8 {
                                format!("{}...{}", &k[..6], &k[k.len() - 4..])
                            } else {
                                "***".to_string()
                            }
                        })
                        .unwrap_or_else(|| "***".to_string());

                    serde_json::json!({
                        "_id": id,
                        "name": name,
                        "key_preview": key,
                        "created_at": created_at,
                        "enabled": enabled
                    })
                })
                .collect();
            Ok(masked)
        }
        Err(e) => Err(format!("Failed to list API keys: {}", e)),
    }
}

/// Revoke (disable) an API key by ID
pub fn revoke_api_key(
    adapter: &Arc<IronBaseAdapter>,
    id: u64,
    cache: &ApiKeyCache,
) -> Result<bool, String> {
    match adapter.update_one(
        "_system.api_keys",
        serde_json::json!({"_id": id}),
        serde_json::json!({"$set": {"enabled": false}}),
    ) {
        Ok(result) => {
            if result.matched_count > 0 {
                cache.invalidate();
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(e) => Err(format!("Failed to revoke API key: {}", e)),
    }
}

/// Delete an API key by ID
pub fn delete_api_key(
    adapter: &Arc<IronBaseAdapter>,
    id: u64,
    cache: &ApiKeyCache,
) -> Result<bool, String> {
    match adapter.delete_one("_system.api_keys", serde_json::json!({"_id": id})) {
        Ok(count) => {
            if count > 0 {
                cache.invalidate();
                Ok(true)
            } else {
                Ok(false)
            }
        }
        Err(e) => Err(format!("Failed to delete API key: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key() {
        let key1 = generate_api_key();
        let key2 = generate_api_key();

        assert!(key1.starts_with("sk-"));
        assert!(key2.starts_with("sk-"));
        assert_eq!(key1.len(), 35); // "sk-" + 32 hex chars
                                    // Keys should be different (high probability)
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_constant_time_compare() {
        assert!(constant_time_compare(b"hello", b"hello"));
        assert!(!constant_time_compare(b"hello", b"world"));
        assert!(!constant_time_compare(b"hello", b"hell"));
        assert!(!constant_time_compare(b"", b"a"));
        assert!(constant_time_compare(b"", b""));
    }
}
