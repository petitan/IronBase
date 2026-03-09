//! Error handling for FFI
//!
//! Provides error codes and thread-local error messages for FFI consumers.
//! Pattern: Functions return error codes, detailed messages available via ironbase_get_last_error()

use ironbase_core::IronBaseError;
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

    /// Operation not allowed (e.g., dropping protected collection)
    OperationNotAllowed = -16,

    /// Database is locked by another process
    DatabaseLocked = -17,

    /// Out of memory
    OutOfMemory = -18,

    /// Operation cancelled by client
    Cancelled = -19,

    /// Operation timed out
    Timeout = -20,

    /// Database is closed
    DatabaseClosed = -21,

    /// Duplicate key error
    DuplicateKey = -22,

    /// Unknown/internal error
    Unknown = -99,
}

impl From<&IronBaseError> for IronBaseErrorCode {
    fn from(err: &IronBaseError) -> Self {
        match err {
            IronBaseError::Io(_) => IronBaseErrorCode::IoError,
            IronBaseError::Serialization(_)
            | IronBaseError::Deserialization(_)
            | IronBaseError::Bincode(_) => IronBaseErrorCode::SerializationError,
            IronBaseError::CollectionNotFound(_) => IronBaseErrorCode::CollectionNotFound,
            IronBaseError::CollectionExists(_) => IronBaseErrorCode::CollectionExists,
            IronBaseError::InvalidCollectionName(_) => IronBaseErrorCode::InvalidQuery,
            IronBaseError::SystemCollectionError(_) => IronBaseErrorCode::OperationNotAllowed,
            IronBaseError::DocumentNotFound => IronBaseErrorCode::DocumentNotFound,
            IronBaseError::InvalidDocumentId(_) | IronBaseError::DocumentValidationFailed(_) => {
                IronBaseErrorCode::InvalidQuery
            }
            IronBaseError::DuplicateKey(_, _) => IronBaseErrorCode::DuplicateKey,
            IronBaseError::InvalidQuery(_)
            | IronBaseError::UnsupportedOperator(_)
            | IronBaseError::QuerySyntaxError(_)
            | IronBaseError::QueryExecutionError(_)
            | IronBaseError::InvalidProjection(_)
            | IronBaseError::InvalidSort(_) => IronBaseErrorCode::InvalidQuery,
            IronBaseError::Corruption(_) => IronBaseErrorCode::Corruption,
            IronBaseError::IndexError(_)
            | IronBaseError::IndexNotFound(_)
            | IronBaseError::IndexExists(_)
            | IronBaseError::ProtectedFieldIndex(_)
            | IronBaseError::CompoundIndexPrefixMismatch { .. }
            | IronBaseError::FuzzyIndexError(_)
            | IronBaseError::FulltextIndexError(_)
            | IronBaseError::VectorIndexError(_)
            | IronBaseError::VectorDimensionMismatch { .. } => IronBaseErrorCode::IndexError,
            IronBaseError::AggregationError(_)
            | IronBaseError::InvalidPipelineStage(_)
            | IronBaseError::InvalidAccumulator(_) => IronBaseErrorCode::AggregationError,
            IronBaseError::AggregationMemoryLimit(_) => IronBaseErrorCode::OutOfMemory,
            IronBaseError::AggregationTimeout => IronBaseErrorCode::Timeout,
            IronBaseError::SchemaError(_)
            | IronBaseError::SchemaNotFound(_)
            | IronBaseError::InvalidSchema(_) => IronBaseErrorCode::SchemaError,
            IronBaseError::TransactionCommitted
            | IronBaseError::TransactionAborted(_)
            | IronBaseError::TransactionNotActive
            | IronBaseError::TransactionConflict(_)
            | IronBaseError::TransactionDeadlock
            | IronBaseError::NestedTransactionNotAllowed => IronBaseErrorCode::TransactionCommitted,
            IronBaseError::WALCorruption
            | IronBaseError::WALWriteError(_)
            | IronBaseError::WALReadError(_)
            | IronBaseError::WALRecoveryFailed(_)
            | IronBaseError::CheckpointFailed(_) => IronBaseErrorCode::WalCorruption,
            IronBaseError::StorageError(_)
            | IronBaseError::FileSystemError(_)
            | IronBaseError::CompactionFailed(_) => IronBaseErrorCode::Corruption,
            IronBaseError::DatabaseLocked(_) => IronBaseErrorCode::DatabaseLocked,
            IronBaseError::DatabaseClosed => IronBaseErrorCode::DatabaseClosed,
            IronBaseError::OperationNotAllowed(_) | IronBaseError::ReadOnlyViolation(_) => {
                IronBaseErrorCode::OperationNotAllowed
            }
            IronBaseError::ResourceExhausted(_) => IronBaseErrorCode::OutOfMemory,
            IronBaseError::InvalidConfiguration(_)
            | IronBaseError::ConfigFileError(_)
            | IronBaseError::InvalidDurabilityMode(_) => IronBaseErrorCode::InvalidQuery,
            IronBaseError::OutOfMemory(_) => IronBaseErrorCode::OutOfMemory,
            IronBaseError::Cancelled(_) => IronBaseErrorCode::Cancelled,
            IronBaseError::Timeout(_) => IronBaseErrorCode::Timeout,
            IronBaseError::Unknown(_) | IronBaseError::InternalError(_) => {
                IronBaseErrorCode::Unknown
            }
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

/// Set error from IronBaseError (internal use)
pub(crate) fn set_error(err: &IronBaseError) -> IronBaseErrorCode {
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
