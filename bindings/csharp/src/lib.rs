//! IronBase C FFI Layer for C# Integration
//!
//! This crate provides a C-compatible FFI layer for IronBase, designed to be
//! consumed by C# via P/Invoke. The csbindgen build script automatically
//! generates the C# NativeMethods.g.cs file.

mod handles;
mod error;
mod database;
mod collection;
mod crud;
mod index;
mod aggregation;
mod transaction;
mod memory;

// Re-export all public FFI functions
pub use handles::*;
pub use error::*;
pub use database::*;
pub use collection::*;
pub use crud::*;
pub use index::*;
pub use aggregation::*;
pub use transaction::*;
pub use memory::*;
