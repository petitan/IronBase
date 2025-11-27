//! Cursor FFI functions for streaming query results
//!
//! Provides memory-efficient iteration over large result sets

use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::handles::{CollHandle, validate_coll_handle};
use crate::error::{IronBaseErrorCode, set_last_error, clear_last_error, c_str_to_string, string_to_c_str};

/// Opaque cursor handle
pub struct CursorState {
    documents: Vec<Value>,
    position: usize,
    batch_size: usize,
}

/// Cursor handle type
pub type CursorHandle = *mut CursorState;

// Global counter for cursor handles
static CURSOR_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Create a cursor for streaming through query results
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `batch_size`: Number of documents per batch
/// - `out_cursor`: Pointer to receive the cursor handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Example
/// ```c
/// CursorHandle cursor;
/// ironbase_create_cursor(coll, "{}", 100, &cursor);
/// while (!ironbase_cursor_is_finished(cursor)) {
///     char* batch = ironbase_cursor_next_batch(cursor);
///     // process batch
///     ironbase_free_string(batch);
/// }
/// ironbase_cursor_release(cursor);
/// ```
#[no_mangle]
pub extern "C" fn ironbase_create_cursor(
    handle: CollHandle,
    query_json: *const c_char,
    batch_size: u32,
    out_cursor: *mut CursorHandle,
) -> i32 {
    clear_last_error();

    if out_cursor.is_null() {
        set_last_error("out_cursor is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let query_str = match c_str_to_string(query_json) {
        Some(s) => s,
        None => {
            set_last_error("Query JSON is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let query: Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid query JSON: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    // Execute query and get all documents
    let documents = match coll.inner.find(&query) {
        Ok(docs) => docs,
        Err(e) => {
            set_last_error(&format!("Query failed: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    let cursor = Box::new(CursorState {
        documents,
        position: 0,
        batch_size: batch_size as usize,
    });

    unsafe {
        *out_cursor = Box::into_raw(cursor);
    }

    let _ = CURSOR_COUNTER.fetch_add(1, Ordering::SeqCst);
    IronBaseErrorCode::Success as i32
}

/// Get the next document from cursor
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - JSON document string (caller must free with `ironbase_free_string()`)
/// - Null if cursor is exhausted or on error
#[no_mangle]
pub extern "C" fn ironbase_cursor_next(cursor: CursorHandle) -> *mut c_char {
    clear_last_error();

    if cursor.is_null() {
        set_last_error("Cursor handle is null");
        return ptr::null_mut();
    }

    let state = unsafe { &mut *cursor };

    if state.position >= state.documents.len() {
        return ptr::null_mut();
    }

    let doc = &state.documents[state.position];
    state.position += 1;

    match serde_json::to_string(doc) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize document: {}", e));
            ptr::null_mut()
        }
    }
}

/// Get the next batch of documents from cursor
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - JSON array of documents (caller must free with `ironbase_free_string()`)
/// - Empty array "[]" if cursor is exhausted
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_cursor_next_batch(cursor: CursorHandle) -> *mut c_char {
    clear_last_error();

    if cursor.is_null() {
        set_last_error("Cursor handle is null");
        return ptr::null_mut();
    }

    let state = unsafe { &mut *cursor };

    if state.position >= state.documents.len() {
        return string_to_c_str("[]");
    }

    let end = (state.position + state.batch_size).min(state.documents.len());
    let batch: Vec<&Value> = state.documents[state.position..end].iter().collect();
    state.position = end;

    match serde_json::to_string(&batch) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize batch: {}", e));
            ptr::null_mut()
        }
    }
}

/// Get a specific chunk of documents from cursor
///
/// # Parameters
/// - `cursor`: The cursor handle
/// - `chunk_size`: Number of documents to retrieve
///
/// # Returns
/// - JSON array of documents (caller must free with `ironbase_free_string()`)
/// - Empty array "[]" if cursor is exhausted
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_cursor_next_chunk(cursor: CursorHandle, chunk_size: u32) -> *mut c_char {
    clear_last_error();

    if cursor.is_null() {
        set_last_error("Cursor handle is null");
        return ptr::null_mut();
    }

    let state = unsafe { &mut *cursor };

    if state.position >= state.documents.len() {
        return string_to_c_str("[]");
    }

    let end = (state.position + chunk_size as usize).min(state.documents.len());
    let chunk: Vec<&Value> = state.documents[state.position..end].iter().collect();
    state.position = end;

    match serde_json::to_string(&chunk) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize chunk: {}", e));
            ptr::null_mut()
        }
    }
}

/// Get remaining document count
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - Number of remaining documents (0 if exhausted or invalid)
#[no_mangle]
pub extern "C" fn ironbase_cursor_remaining(cursor: CursorHandle) -> u64 {
    if cursor.is_null() {
        return 0;
    }

    let state = unsafe { &*cursor };
    state.documents.len().saturating_sub(state.position) as u64
}

/// Get total document count
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - Total number of documents in cursor (0 if invalid)
#[no_mangle]
pub extern "C" fn ironbase_cursor_total(cursor: CursorHandle) -> u64 {
    if cursor.is_null() {
        return 0;
    }

    let state = unsafe { &*cursor };
    state.documents.len() as u64
}

/// Get current position
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - Current position (0 if invalid)
#[no_mangle]
pub extern "C" fn ironbase_cursor_position(cursor: CursorHandle) -> u64 {
    if cursor.is_null() {
        return 0;
    }

    let state = unsafe { &*cursor };
    state.position as u64
}

/// Check if cursor is exhausted
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - 1 if exhausted, 0 if not (also returns 1 for invalid cursor)
#[no_mangle]
pub extern "C" fn ironbase_cursor_is_finished(cursor: CursorHandle) -> i32 {
    if cursor.is_null() {
        return 1;
    }

    let state = unsafe { &*cursor };
    if state.position >= state.documents.len() { 1 } else { 0 }
}

/// Reset cursor to the beginning
///
/// # Parameters
/// - `cursor`: The cursor handle
#[no_mangle]
pub extern "C" fn ironbase_cursor_rewind(cursor: CursorHandle) {
    if cursor.is_null() {
        return;
    }

    let state = unsafe { &mut *cursor };
    state.position = 0;
}

/// Skip the next N documents
///
/// # Parameters
/// - `cursor`: The cursor handle
/// - `n`: Number of documents to skip
#[no_mangle]
pub extern "C" fn ironbase_cursor_skip(cursor: CursorHandle, n: u64) {
    if cursor.is_null() {
        return;
    }

    let state = unsafe { &mut *cursor };
    state.position = (state.position + n as usize).min(state.documents.len());
}

/// Collect all remaining documents
///
/// # Parameters
/// - `cursor`: The cursor handle
///
/// # Returns
/// - JSON array of all remaining documents (caller must free with `ironbase_free_string()`)
/// - Empty array "[]" if cursor is exhausted
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_cursor_collect_all(cursor: CursorHandle) -> *mut c_char {
    clear_last_error();

    if cursor.is_null() {
        set_last_error("Cursor handle is null");
        return ptr::null_mut();
    }

    let state = unsafe { &mut *cursor };

    if state.position >= state.documents.len() {
        return string_to_c_str("[]");
    }

    let remaining: Vec<&Value> = state.documents[state.position..].iter().collect();
    state.position = state.documents.len();

    match serde_json::to_string(&remaining) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize documents: {}", e));
            ptr::null_mut()
        }
    }
}

/// Release a cursor handle
///
/// # Parameters
/// - `cursor`: The cursor handle to release
///
/// # Safety
/// - The handle must have been created by `ironbase_create_cursor()`
/// - The handle must not be used after this call
/// - It is safe to call with a null handle (no-op)
#[no_mangle]
pub extern "C" fn ironbase_cursor_release(cursor: CursorHandle) {
    if cursor.is_null() {
        return;
    }

    // Take ownership and drop
    let _ = unsafe { Box::from_raw(cursor) };
}
