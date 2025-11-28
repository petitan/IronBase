//! Error handling for FFI
//!
//! Provides error codes and thread-local error messages for FFI consumers.
//! Pattern: Functions return error codes, detailed messages available via ironbase_get_last_error()

use ironbase_core::MongoLiteError;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Error codes returned by FFI functions
///
/// These map to IronBaseErrorCode enum in C#
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IronBaseErrorCode {
    /// Operation succeeded
    Success = 0,

    /// Null pointer passed to function
    NullPointer = -1,

    /// Invalid handle (already closed or corrupted)
    InvalidHandle = -2,

    /// I/O error (file system, etc.)
    IoError = -3,

    /// Serialization/deserialization error
    SerializationError = -4,

    /// Collection not found
    CollectionNotFound = -5,

    /// Collection already exists
    CollectionExists = -6,

    /// Document not found
    DocumentNotFound = -7,

    /// Invalid query syntax
    InvalidQuery = -8,

    /// Database corruption detected
    Corruption = -9,

    /// Index operation error
    IndexError = -10,

    /// Aggregation pipeline error
    AggregationError = -11,

    /// Schema validation failed
    SchemaError = -12,

    /// Transaction already committed/aborted
    TransactionCommitted = -13,

    /// Transaction aborted
    TransactionAborted = -14,

    /// WAL corruption detected
    WalCorruption = -15,

    /// Unknown/internal error
    Unknown = -99,
}

impl From<&MongoLiteError> for IronBaseErrorCode {
    fn from(err: &MongoLiteError) -> Self {
        match err {
            MongoLiteError::Io(_) => IronBaseErrorCode::IoError,
            MongoLiteError::Serialization(_) => IronBaseErrorCode::SerializationError,
            MongoLiteError::Deserialization(_) => IronBaseErrorCode::SerializationError,
            MongoLiteError::CollectionNotFound(_) => IronBaseErrorCode::CollectionNotFound,
            MongoLiteError::CollectionExists(_) => IronBaseErrorCode::CollectionExists,
            MongoLiteError::DocumentNotFound => IronBaseErrorCode::DocumentNotFound,
            MongoLiteError::InvalidQuery(_) => IronBaseErrorCode::InvalidQuery,
            MongoLiteError::Corruption(_) => IronBaseErrorCode::Corruption,
            MongoLiteError::IndexError(_) => IronBaseErrorCode::IndexError,
            MongoLiteError::AggregationError(_) => IronBaseErrorCode::AggregationError,
            MongoLiteError::SchemaError(_) => IronBaseErrorCode::SchemaError,
            MongoLiteError::TransactionCommitted => IronBaseErrorCode::TransactionCommitted,
            MongoLiteError::TransactionAborted(_) => IronBaseErrorCode::TransactionAborted,
            MongoLiteError::WALCorruption => IronBaseErrorCode::WalCorruption,
            MongoLiteError::Unknown(_) => IronBaseErrorCode::Unknown,
        }
    }
}

// Thread-local storage for the last error message
thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Set the last error message (internal use)
pub(crate) fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Set error from MongoLiteError (internal use)
pub(crate) fn set_error(err: &MongoLiteError) -> IronBaseErrorCode {
    set_last_error(&err.to_string());
    IronBaseErrorCode::from(err)
}

/// Clear the last error message
pub(crate) fn clear_last_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

/// Get the last error message
///
/// Returns a pointer to a null-terminated UTF-8 string.
/// The pointer is valid until the next FFI call on the same thread.
/// Returns null if no error has occurred.
///
/// # Safety
/// The returned pointer must not be freed by the caller.
/// The pointer is only valid until the next FFI call on the same thread.
#[no_mangle]
pub extern "C" fn ironbase_get_last_error() -> *const c_char {
    LAST_ERROR.with(|e| match e.borrow().as_ref() {
        Some(cstr) => cstr.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Clear the last error message
///
/// Call this before a sequence of operations if you want to check
/// for errors after the sequence.
#[no_mangle]
pub extern "C" fn ironbase_clear_error() {
    clear_last_error();
}

/// Check if an error occurred
///
/// Returns 1 if there is an error message, 0 otherwise.
#[no_mangle]
pub extern "C" fn ironbase_has_error() -> i32 {
    LAST_ERROR.with(|e| if e.borrow().is_some() { 1 } else { 0 })
}

/// Helper to convert C string to Rust string
///
/// Returns None if the pointer is null or the string is not valid UTF-8
pub(crate) fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

/// Helper to convert Rust string to C string (caller must free with ironbase_free_string)
pub(crate) fn string_to_c_str(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cstr) => cstr.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
