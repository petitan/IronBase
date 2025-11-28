//! Collection handle management FFI functions

use std::os::raw::c_char;
use std::sync::Arc;

use crate::error::{
    c_str_to_string, clear_last_error, set_error, set_last_error, IronBaseErrorCode,
};
use crate::handles::{validate_db_handle, CollHandle, CollectionHandle, DbHandle};

/// Get or create a collection
///
/// # Parameters
/// - `db_handle`: The database handle
/// - `name`: Collection name (UTF-8 null-terminated string)
/// - `out_handle`: Pointer to receive the collection handle
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Safety
/// - The returned handle must be released with `ironbase_collection_release()`
/// - The database handle must remain valid while the collection handle is in use
#[no_mangle]
pub extern "C" fn ironbase_collection(
    db_handle: DbHandle,
    name: *const c_char,
    out_handle: *mut CollHandle,
) -> i32 {
    clear_last_error();

    if out_handle.is_null() {
        set_last_error("out_handle is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let db = match validate_db_handle(db_handle) {
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

    match db.inner.collection(&name_str) {
        Ok(coll) => {
            let handle = Box::new(CollectionHandle::new(coll, Arc::clone(&db.inner), name_str));
            unsafe {
                *out_handle = Box::into_raw(handle);
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Release a collection handle
///
/// # Parameters
/// - `handle`: The collection handle to release
///
/// # Safety
/// - The handle must have been created by `ironbase_collection()`
/// - The handle must not be used after this call
/// - It is safe to call with a null handle (no-op)
#[no_mangle]
pub extern "C" fn ironbase_collection_release(handle: CollHandle) {
    clear_last_error();

    if !handle.is_null() {
        // Take ownership and drop
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}

/// Get the collection name
///
/// # Parameters
/// - `handle`: The collection handle
///
/// # Returns
/// - Pointer to the name string (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_collection_name(handle: CollHandle) -> *mut c_char {
    clear_last_error();

    let coll = match crate::handles::validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return std::ptr::null_mut();
        }
    };

    crate::error::string_to_c_str(&coll.name)
}

/// Count documents in the collection
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string (use "{}" for all documents)
/// - `out_count`: Pointer to receive the count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_count_documents(
    handle: CollHandle,
    query_json: *const c_char,
    out_count: *mut u64,
) -> i32 {
    clear_last_error();

    if out_count.is_null() {
        set_last_error("out_count is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let coll = match crate::handles::validate_coll_handle(handle) {
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

    let query: serde_json::Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    match coll.inner.count_documents(&query) {
        Ok(count) => {
            unsafe {
                *out_count = count;
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Get distinct values for a field
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field name (UTF-8 null-terminated string)
/// - `query_json`: Query filter as JSON string (use "{}" for all documents)
///
/// # Returns
/// - JSON array of distinct values (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_distinct(
    handle: CollHandle,
    field: *const c_char,
    query_json: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match crate::handles::validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return std::ptr::null_mut();
        }
    };

    let field_str = match c_str_to_string(field) {
        Some(s) => s,
        None => {
            set_last_error("Field name is null or invalid UTF-8");
            return std::ptr::null_mut();
        }
    };

    let query_str = match c_str_to_string(query_json) {
        Some(s) => s,
        None => {
            set_last_error("Query JSON is null or invalid UTF-8");
            return std::ptr::null_mut();
        }
    };

    let query: serde_json::Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return std::ptr::null_mut();
        }
    };

    match coll.inner.distinct(&field_str, &query) {
        Ok(values) => match serde_json::to_string(&values) {
            Ok(json) => crate::error::string_to_c_str(&json),
            Err(e) => {
                set_last_error(&format!("Failed to serialize values: {}", e));
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            set_error(&e);
            std::ptr::null_mut()
        }
    }
}
