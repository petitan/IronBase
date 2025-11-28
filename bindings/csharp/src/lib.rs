//! IronBase C FFI Layer for C# Integration
//!
//! This crate provides a C-compatible FFI layer for IronBase, designed to be
//! consumed by C# via P/Invoke. The csbindgen build script automatically
//! generates the C# NativeMethods.g.cs file.

// FFI code intentionally dereferences raw pointers passed from C#
// This is safe because C# guarantees these pointers are valid during the call
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// Some FFI helper functions are reserved for future use
#![allow(dead_code)]

mod aggregation;
mod collection;
mod crud;
mod cursor;
mod database;
mod error;
mod handles;
mod index;
mod logging;
mod memory;
mod schema;
mod transaction;

// Re-export all public FFI functions
pub use aggregation::*;
pub use collection::*;
pub use crud::*;
pub use cursor::*;
pub use database::*;
pub use error::*;
pub use handles::*;
pub use index::*;
pub use logging::*;
pub use memory::*;
pub use schema::*;
pub use transaction::*;
