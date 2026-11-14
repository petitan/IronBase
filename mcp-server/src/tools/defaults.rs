//! Common default values for MCP tools
//!
//! The constants and serde default functions the tool arguments need live in
//! `ironbase_mcp_types::defaults` (shared with clients) and are re-exported here;
//! this module adds the server-only ones.

pub use ironbase_mcp_types::defaults::*;

// ============================================================================
// Batch Processing (server configuration)
// ============================================================================

/// Configurable batch size override (set from [embedding].batch_size config)
static CONFIGURED_BATCH_SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Set the batch size from config (call once at startup)
pub fn set_configured_batch_size(size: usize) {
    let _ = CONFIGURED_BATCH_SIZE.set(size);
}

/// Get the effective batch size (config override > default)
pub fn effective_batch_size() -> usize {
    CONFIGURED_BATCH_SIZE
        .get()
        .copied()
        .unwrap_or(DEFAULT_BATCH_SIZE)
}

// ============================================================================
// Search Fusion (engine limit)
// ============================================================================

/// Maximum internal limit for search fusion (caps per-source fetch before RRF):
/// the engine's hybrid candidate cap
pub const MAX_INTERNAL_LIMIT: usize = ironbase_core::search::hybrid::MAX_CANDIDATES;
