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

/// The in-flight tracking key of a client's JSON-RPC request id. Ids are
/// only unique per client (and often sequential): keyed by id alone, one
/// client's request replaced another's with the same id, and any caller
/// could cancel another client's request by guessing its id.
pub(crate) fn tracked_request_id(client: &str, request_id: &str) -> String {
    format!("{client}\u{1f}{request_id}")
}

/// The request id a `notifications/cancelled` names, as tracked: MCP's
/// `RequestId` is a string or a number (a number is tracked by its decimal
/// form, like the request's own id); `request_id` is accepted too
pub(crate) fn cancelled_request_id(params: &serde_json::Value) -> Option<String> {
    let id = params
        .get("requestId")
        .or_else(|| params.get("request_id"))?;
    match id {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    /// A client that numbers its requests cancels them by number (the TUI
    /// does): only a string id used to be found
    #[test]
    fn test_cancelled_request_id_takes_strings_and_numbers() {
        use serde_json::json;
        assert_eq!(
            cancelled_request_id(&json!({"requestId": 42})).as_deref(),
            Some("42")
        );
        assert_eq!(
            cancelled_request_id(&json!({"requestId": "req-1"})).as_deref(),
            Some("req-1")
        );
        assert_eq!(
            cancelled_request_id(&json!({"request_id": 7})).as_deref(),
            Some("7")
        );
        assert_eq!(cancelled_request_id(&json!({"requestId": null})), None);
        assert_eq!(cancelled_request_id(&json!({})), None);
    }

    use super::*;

    /// Two clients' requests with the same JSON-RPC id are tracked apart,
    /// so neither can cancel or replace the other's
    #[test]
    fn same_request_id_of_two_clients_is_tracked_apart() {
        let a = client_identity(None, Some("10.0.0.1:1".parse().unwrap()));
        let b = client_identity(Some("key"), Some("10.0.0.1:1".parse().unwrap()));
        assert_ne!(tracked_request_id(&a, "1"), tracked_request_id(&b, "1"));
        assert_eq!(tracked_request_id(&a, "1"), tracked_request_id(&a, "1"));

        let tracker = crate::cancellation::RequestTracker::new();
        let flag = tracker.start(&tracked_request_id(&a, "1"));
        assert!(!tracker.cancel(&tracked_request_id(&b, "1"), None));
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(tracker.cancel(&tracked_request_id(&a, "1"), None));
    }
}
