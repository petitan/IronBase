//! Client identity and initialization tracking utilities

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Mutex;

/// Generate a unique client identity string from API key or IP address
pub(crate) fn client_identity(api_key: Option<&str>, remote_addr: Option<SocketAddr>) -> String {
    if let Some(key) = api_key {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        return format!("api:{:x}", hasher.finish());
    }

    remote_addr
        .map(|addr| format!("ip:{}", addr.ip()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Check if a client has been initialized (MCP lifecycle)
pub(crate) fn is_client_initialized(
    initialized_clients: &Mutex<HashSet<String>>,
    client_id: &str,
) -> bool {
    match initialized_clients.lock() {
        Ok(set) => set.contains(client_id),
        Err(poisoned) => {
            // Recover from poisoned mutex - log warning and check the data
            tracing::warn!("Client initialized set mutex was poisoned, recovering");
            poisoned.into_inner().contains(client_id)
        }
    }
}

/// Mark a client as initialized (MCP lifecycle)
pub(crate) fn mark_client_initialized(
    initialized_clients: &Mutex<HashSet<String>>,
    client_id: &str,
) {
    match initialized_clients.lock() {
        Ok(mut set) => {
            set.insert(client_id.to_string());
        }
        Err(poisoned) => {
            // Recover from poisoned mutex - log warning and insert anyway
            tracing::warn!("Client initialized set mutex was poisoned, recovering");
            poisoned.into_inner().insert(client_id.to_string());
        }
    }
}
