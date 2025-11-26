//! Database lifecycle FFI functions
//!
//! Provides functions to open, close, flush, and manage database instances.

use std::os::raw::c_char;
use std::ptr;

use ironbase_core::{DatabaseCore, DurabilityMode};

use crate::handles::{DatabaseHandle, DbHandle};
use crate::error::{IronBaseErrorCode, set_last_error, set_error, clear_last_error, c_str_to_string, string_to_c_str};

/// Open a database file
///
/// # Parameters
/// - `path`: Path to the database file (UTF-8 null-terminated string)
/// - `out_handle`: Pointer to receive the database handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure (check `ironbase_get_last_error()` for details)
///
/// # Safety
/// - `path` must be a valid null-terminated UTF-8 string
/// - `out_handle` must be a valid pointer to a DbHandle
/// - The returned handle must be closed with `ironbase_close()`
#[no_mangle]
pub extern "C" fn ironbase_open(
    path: *const c_char,
    out_handle: *mut DbHandle,
) -> i32 {
    clear_last_error();

    // Validate parameters
    if out_handle.is_null() {
        set_last_error("out_handle is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let path_str = match c_str_to_string(path) {
        Some(s) => s,
        None => {
            set_last_error("path is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    // Open database with safe durability (default)
    match DatabaseCore::open(&path_str) {
        Ok(db) => {
            let handle = Box::new(DatabaseHandle::new(db));
            unsafe {
                *out_handle = Box::into_raw(handle);
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => {
            set_error(&e) as i32
        }
    }
}

/// Open a database file with specific durability mode
///
/// # Parameters
/// - `path`: Path to the database file (UTF-8 null-terminated string)
/// - `durability_mode`: 0=Safe, 1=Batch, 2=Unsafe
/// - `batch_size`: Batch size for Batch mode (ignored for other modes)
/// - `out_handle`: Pointer to receive the database handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_open_with_durability(
    path: *const c_char,
    durability_mode: i32,
    batch_size: u32,
    out_handle: *mut DbHandle,
) -> i32 {
    clear_last_error();

    if out_handle.is_null() {
        set_last_error("out_handle is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let path_str = match c_str_to_string(path) {
        Some(s) => s,
        None => {
            set_last_error("path is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let mode = match durability_mode {
        0 => DurabilityMode::Safe,
        1 => DurabilityMode::Batch { batch_size: batch_size as usize },
        2 => DurabilityMode::unsafe_manual(),
        _ => {
            set_last_error("Invalid durability mode (must be 0=Safe, 1=Batch, 2=Unsafe)");
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    match DatabaseCore::open_with_durability(&path_str, mode) {
        Ok(db) => {
            let handle = Box::new(DatabaseHandle::new(db));
            unsafe {
                *out_handle = Box::into_raw(handle);
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => {
            set_error(&e) as i32
        }
    }
}

/// Close a database handle
///
/// This flushes all pending data and releases resources.
/// After this call, the handle is invalid and must not be used.
///
/// # Parameters
/// - `handle`: The database handle to close
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Safety
/// - The handle must have been created by `ironbase_open()`
/// - The handle must not be used after this call
/// - It is safe to call with a null handle (no-op)
#[no_mangle]
pub extern "C" fn ironbase_close(handle: DbHandle) -> i32 {
    clear_last_error();

    if handle.is_null() {
        return IronBaseErrorCode::Success as i32;
    }

    // Take ownership and drop
    let db = unsafe { Box::from_raw(handle) };

    // Flush before dropping
    match db.inner.flush() {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Flush all pending data to disk
///
/// # Parameters
/// - `handle`: The database handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_flush(handle: DbHandle) -> i32 {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    match db.inner.flush() {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Checkpoint the database (clear WAL)
///
/// # Parameters
/// - `handle`: The database handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_checkpoint(handle: DbHandle) -> i32 {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    match db.inner.checkpoint() {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Get database statistics as JSON
///
/// # Parameters
/// - `handle`: The database handle
///
/// # Returns
/// - Pointer to a JSON string (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_stats(handle: DbHandle) -> *mut c_char {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return ptr::null_mut();
        }
    };

    let stats = db.inner.stats();
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize stats: {}", e));
            ptr::null_mut()
        }
    }
}

/// Get the database file path
///
/// # Parameters
/// - `handle`: The database handle
///
/// # Returns
/// - Pointer to the path string (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_path(handle: DbHandle) -> *mut c_char {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return ptr::null_mut();
        }
    };

    string_to_c_str(db.inner.path())
}

/// Compact the database (remove tombstones)
///
/// # Parameters
/// - `handle`: The database handle
/// - `out_stats`: Pointer to receive compaction stats JSON (optional, can be null)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_compact(
    handle: DbHandle,
    out_stats: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    match db.inner.compact() {
        Ok(stats) => {
            if !out_stats.is_null() {
                // Build JSON manually since CompactionStats doesn't derive Serialize
                let stats_json = serde_json::json!({
                    "size_before": stats.size_before,
                    "size_after": stats.size_after,
                    "space_saved": stats.space_saved(),
                    "documents_scanned": stats.documents_scanned,
                    "documents_kept": stats.documents_kept,
                    "tombstones_removed": stats.tombstones_removed,
                    "peak_memory_mb": stats.peak_memory_mb,
                    "compression_ratio": stats.compression_ratio()
                });
                if let Ok(json) = serde_json::to_string_pretty(&stats_json) {
                    unsafe {
                        *out_stats = string_to_c_str(&json);
                    }
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// List all collections in the database
///
/// # Parameters
/// - `handle`: The database handle
///
/// # Returns
/// - JSON array of collection names (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_list_collections(handle: DbHandle) -> *mut c_char {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return ptr::null_mut();
        }
    };

    let collections = db.inner.list_collections();
    match serde_json::to_string(&collections) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize collection list: {}", e));
            ptr::null_mut()
        }
    }
}

/// Drop a collection from the database
///
/// # Parameters
/// - `handle`: The database handle
/// - `name`: Collection name (UTF-8 null-terminated string)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_drop_collection(
    handle: DbHandle,
    name: *const c_char,
) -> i32 {
    clear_last_error();

    let db = match crate::handles::validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let name_str = match c_str_to_string(name) {
        Some(s) => s,
        None => {
            set_last_error("Collection name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    match db.inner.drop_collection(&name_str) {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}
