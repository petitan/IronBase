//! HTTP server application state

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared state for HTTP server routes
pub(crate) struct HttpAppState {
    /// Central service layer for tool execution
    pub(crate) service: Arc<crate::IronBaseService>,
    /// MCP lifecycle state: track initialize per client
    /// Per spec: "The initialization phase MUST be the first interaction"
    pub(crate) initialized_clients: Mutex<HashSet<String>>,
    /// Timeout for long-running tool operations (global default)
    pub(crate) tool_timeout: Duration,
    /// In-flight request tracker for MCP notifications/cancelled support
    pub(crate) request_tracker: Arc<crate::cancellation::RequestTracker>,
}
