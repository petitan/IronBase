//! Shared types of the IronBase MCP tools
//!
//! One definition of each tool's wire format, used by the server
//! (`mcp-ironbase-server`) and by its clients, so the two cannot drift apart:
//! - `params` - Argument structs (what a tool accepts)
//! - `defaults` - Default constants and serde default functions of the arguments
//! - `tools` - The `Tool` trait and one marker type per tool (name, arguments, response)

pub mod defaults;
pub mod params;
pub mod tools;

pub use tools::Tool;
