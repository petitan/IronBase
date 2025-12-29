//! IronBase MCP Server - Model Context Protocol server for IronBase document database

#![recursion_limit = "512"]

pub mod acl;
pub mod adapter;
pub mod api_keys;
pub mod engine;
pub mod error;
pub mod http_server;
pub mod listener;
pub mod monitoring;
pub mod prompts;
pub mod resources;
pub mod scripting;
pub mod service;
pub mod shutdown;
pub mod tools;

// Re-export main types
pub use acl::{AclConfig, AclManager, CallerContext, InterfaceType, RequiredPermission};
pub use adapter::{FindOptions, IronBaseAdapter, UpdateResult};
pub use api_keys::ApiKeyCache;
pub use engine::{IronBaseService, ServiceContext, ToolRequest, ToolResult};
pub use error::{McpError, OperationContext, Result};
pub use listener::{ListenerConfig, ListenerManager, SYSTEM_LISTENERS_COLLECTION};
pub use prompts::{get_prompt_content, get_prompts_list};
pub use resources::{get_resources_list, read_resource};
pub use scripting::{RhaiEngine, ScriptManager, ScriptResult};
pub use tools::{dispatch_tool, get_tools_list, get_tools_list_filtered};
pub use monitoring::{get_memory_stats, health_check, log_memory_stats, HealthCheck, MemoryStats};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Library name
pub const NAME: &str = "mcp-ironbase-server";

/// Server runtime information for db_stats
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    /// Protocol: "http" or "https"
    pub protocol: String,
    /// Server host
    pub host: String,
    /// Server port
    pub port: u16,
    /// Whether API key authentication is required
    pub require_api_key: bool,
}
