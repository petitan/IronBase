//! IronBase TUI library - everything except the binary's CLI, terminal
//! setup and event loop (`main.rs`)

pub mod app;
pub mod base64_detect;
pub mod config;
pub mod db;
pub mod handlers;
pub mod mcp;
pub mod modals;
pub mod panes;
pub mod state;
#[cfg(test)]
pub(crate) mod testing;
pub mod theme;
pub mod ui;
