//! MCP (Model Context Protocol) client module for IronBase TUI
//!
//! This module provides async communication with an IronBase MCP server,
//! supporting both HTTP and stdio transport modes.

mod client;
mod error;
mod protocol;
mod transport;

pub use client::McpClient;
pub use error::{McpError, McpResult};
pub use protocol::{JsonRpcRequest, JsonRpcResponse, ToolCallResult};
pub use transport::{HttpTransport, StdioTransport, Transport};
