//! IronBase MCP Server - Model Context Protocol server for IronBase document database

#![recursion_limit = "512"]

pub mod adapter;
pub mod error;
pub mod http_server;
pub mod prompts;
pub mod scripting;
pub mod service;
pub mod shutdown;
pub mod tools;

// Re-export main types
pub use adapter::{FindOptions, IronBaseAdapter, UpdateResult};
pub use error::{McpError, Result};
pub use prompts::{get_prompt_content, get_prompts_list};
pub use scripting::{RhaiEngine, ScriptManager, ScriptResult};
pub use tools::{dispatch_tool, get_tools_list};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Library name
pub const NAME: &str = "mcp-ironbase-server";
