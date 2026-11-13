//! MCP (Model Context Protocol) client module for IronBase TUI
//!
//! This module provides async communication with an IronBase MCP server,
//! supporting both HTTP and stdio transport modes.

mod client;
mod error;
mod protocol;
mod transport;

pub use client::McpClient;
pub use error::McpResult;
pub use protocol::{JsonRpcRequest, JsonRpcResponse};
pub use transport::Transport;
