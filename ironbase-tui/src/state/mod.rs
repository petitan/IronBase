//! Application state module
//!
//! This module contains all the state structs and enums for the TUI application.
//! The main App struct remains in app.rs and imports from this module.

pub mod types;
pub mod search;
pub mod filter;
pub mod insert;
pub mod export;
pub mod database;
pub mod index;
pub mod fulltext;
pub mod query;
pub mod script;
pub mod vector;
pub mod rag;

// Re-export all types for convenience
pub use types::*;
pub use search::*;
pub use filter::*;
pub use insert::*;
pub use export::*;
pub use database::*;
pub use index::*;
pub use fulltext::*;
pub use query::*;
pub use script::*;
pub use vector::*;
pub use rag::*;
