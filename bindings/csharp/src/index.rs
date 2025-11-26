//! Index management FFI functions

use std::os::raw::c_char;
use std::ptr;

use crate::handles::{CollHandle, validate_coll_handle};
use crate::error::{IronBaseErrorCode, set_last_error, set_error, clear_last_error, c_str_to_string, string_to_c_str};

/// Create an index on a field
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field name to index (UTF-8 null-terminated string)
/// - `unique`: Whether the index enforces uniqueness (1 = unique, 0 = not unique)
/// - `out_name`: Pointer to receive the index name (optional)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_create_index(
    handle: CollHandle,
    field: *const c_char,
    unique: i32,
    out_name: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let field_str = match c_str_to_string(field) {
        Some(s) => s,
        None => {
            set_last_error("Field name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    match coll.inner.create_index(field_str, unique != 0) {
        Ok(name) => {
            if !out_name.is_null() {
                unsafe {
                    *out_name = string_to_c_str(&name);
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Create a compound index on multiple fields
///
/// # Parameters
/// - `handle`: The collection handle
/// - `fields_json`: JSON array of field names (e.g., ["country", "city"])
/// - `unique`: Whether the index enforces uniqueness
/// - `out_name`: Pointer to receive the index name (optional)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_create_compound_index(
    handle: CollHandle,
    fields_json: *const c_char,
    unique: i32,
    out_name: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let fields_str = match c_str_to_string(fields_json) {
        Some(s) => s,
        None => {
            set_last_error("Fields JSON is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let fields: Vec<String> = match serde_json::from_str(&fields_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid fields JSON: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    if fields.is_empty() {
        set_last_error("Compound index must have at least one field");
        return IronBaseErrorCode::InvalidQuery as i32;
    }

    match coll.inner.create_compound_index(fields, unique != 0) {
        Ok(name) => {
            if !out_name.is_null() {
                unsafe {
                    *out_name = string_to_c_str(&name);
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Drop an index
///
/// # Parameters
/// - `handle`: The collection handle
/// - `index_name`: Index name to drop (UTF-8 null-terminated string)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_drop_index(
    handle: CollHandle,
    index_name: *const c_char,
) -> i32 {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let name_str = match c_str_to_string(index_name) {
        Some(s) => s,
        None => {
            set_last_error("Index name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    match coll.inner.drop_index(&name_str) {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// List all indexes in a collection
///
/// # Parameters
/// - `handle`: The collection handle
///
/// # Returns
/// - JSON array of index names (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_list_indexes(handle: CollHandle) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let indexes = coll.inner.list_indexes();
    match serde_json::to_string(&indexes) {
        Ok(json) => string_to_c_str(&json),
        Err(e) => {
            set_last_error(&format!("Failed to serialize index list: {}", e));
            ptr::null_mut()
        }
    }
}

/// Explain query execution plan
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
///
/// # Returns
/// - JSON query plan (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_explain(
    handle: CollHandle,
    query_json: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let query_str = match c_str_to_string(query_json) {
        Some(s) => s,
        None => {
            set_last_error("Query JSON is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let query: serde_json::Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return ptr::null_mut();
        }
    };

    match coll.inner.explain(&query) {
        Ok(plan) => {
            match serde_json::to_string_pretty(&plan) {
                Ok(json) => string_to_c_str(&json),
                Err(e) => {
                    set_last_error(&format!("Failed to serialize plan: {}", e));
                    ptr::null_mut()
                }
            }
        }
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

/// Find documents with index hint
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `hint`: Index name to use
///
/// # Returns
/// - JSON array of documents (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_find_with_hint(
    handle: CollHandle,
    query_json: *const c_char,
    hint: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let query_str = match c_str_to_string(query_json) {
        Some(s) => s,
        None => {
            set_last_error("Query JSON is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let hint_str = match c_str_to_string(hint) {
        Some(s) => s,
        None => {
            set_last_error("Hint is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let query: serde_json::Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return ptr::null_mut();
        }
    };

    match coll.inner.find_with_hint(&query, &hint_str) {
        Ok(docs) => {
            match serde_json::to_string(&docs) {
                Ok(json) => string_to_c_str(&json),
                Err(e) => {
                    set_last_error(&format!("Failed to serialize results: {}", e));
                    ptr::null_mut()
                }
            }
        }
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}
