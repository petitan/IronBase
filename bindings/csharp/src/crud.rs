//! CRUD operations FFI functions
//!
//! Insert, Find, Update, Delete operations

use std::collections::HashMap;
use std::os::raw::c_char;
use std::ptr;

use ironbase_core::DocumentId;
use serde_json::Value;

use crate::error::{
    c_str_to_string, clear_last_error, set_error, set_last_error, string_to_c_str,
    IronBaseErrorCode,
};
use crate::handles::{validate_coll_handle, CollHandle};

// ============== INSERT ==============

/// Insert one document
///
/// # Parameters
/// - `handle`: The collection handle
/// - `document_json`: Document as JSON string
/// - `out_id`: Pointer to receive the inserted ID as JSON string
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_insert_one(
    handle: CollHandle,
    document_json: *const c_char,
    out_id: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let doc_str = match c_str_to_string(document_json) {
        Some(s) => s,
        None => {
            set_last_error("Document JSON is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let doc: Value = match serde_json::from_str(&doc_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    // Convert JSON Value to HashMap
    let doc_map = match json_to_hashmap(&doc) {
        Some(m) => m,
        None => {
            set_last_error("Document must be a JSON object");
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    // Use database-level insert for proper durability
    match coll.db.insert_one(&coll.name, doc_map) {
        Ok(id) => {
            if !out_id.is_null() {
                let id_json = document_id_to_json(&id);
                unsafe {
                    *out_id = string_to_c_str(&id_json);
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Insert many documents
///
/// # Parameters
/// - `handle`: The collection handle
/// - `documents_json`: JSON array of documents
/// - `out_result`: Pointer to receive result JSON (inserted_count, inserted_ids)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_insert_many(
    handle: CollHandle,
    documents_json: *const c_char,
    out_result: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let docs_str = match c_str_to_string(documents_json) {
        Some(s) => s,
        None => {
            set_last_error("Documents JSON is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let docs: Value = match serde_json::from_str(&docs_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    let docs_array = match docs.as_array() {
        Some(arr) => arr,
        None => {
            set_last_error("Documents must be a JSON array");
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    let mut doc_maps = Vec::with_capacity(docs_array.len());
    for doc in docs_array {
        match json_to_hashmap(doc) {
            Some(m) => doc_maps.push(m),
            None => {
                set_last_error("Each document must be a JSON object");
                return IronBaseErrorCode::SerializationError as i32;
            }
        }
    }

    match coll.db.insert_many(&coll.name, doc_maps) {
        Ok(inserted_ids) => {
            if !out_result.is_null() {
                let ids: Vec<Value> = inserted_ids
                    .iter()
                    .map(|id| serde_json::from_str(&document_id_to_json(id)).unwrap_or(Value::Null))
                    .collect();

                let result_json = serde_json::json!({
                    "acknowledged": true,
                    "inserted_count": inserted_ids.len(),
                    "inserted_ids": ids
                });

                if let Ok(json) = serde_json::to_string(&result_json) {
                    unsafe {
                        *out_result = string_to_c_str(&json);
                    }
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

// ============== FIND ==============

/// Find documents
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
///
/// # Returns
/// - JSON array of documents (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_find(handle: CollHandle, query_json: *const c_char) -> *mut c_char {
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

    let query: Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return ptr::null_mut();
        }
    };

    match coll.inner.find(&query) {
        Ok(docs) => match serde_json::to_string(&docs) {
            Ok(json) => string_to_c_str(&json),
            Err(e) => {
                set_last_error(&format!("Failed to serialize results: {}", e));
                ptr::null_mut()
            }
        },
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

/// Find one document
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
///
/// # Returns
/// - JSON document (caller must free with `ironbase_free_string()`)
/// - Null if not found or on error
#[no_mangle]
pub extern "C" fn ironbase_find_one(handle: CollHandle, query_json: *const c_char) -> *mut c_char {
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

    let query: Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid JSON: {}", e));
            return ptr::null_mut();
        }
    };

    match coll.inner.find_one(&query) {
        Ok(Some(doc)) => match serde_json::to_string(&doc) {
            Ok(json) => string_to_c_str(&json),
            Err(e) => {
                set_last_error(&format!("Failed to serialize document: {}", e));
                ptr::null_mut()
            }
        },
        Ok(None) => ptr::null_mut(),
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

/// Find documents with options (projection, sort, limit, skip)
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `options_json`: Options as JSON: {"projection": {...}, "sort": [[field, 1/-1], ...], "limit": n, "skip": n}
///
/// # Returns
/// - JSON array of documents (caller must free with `ironbase_free_string()`)
/// - Null on error
#[no_mangle]
pub extern "C" fn ironbase_find_with_options(
    handle: CollHandle,
    query_json: *const c_char,
    options_json: *const c_char,
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

    let options_str = match c_str_to_string(options_json) {
        Some(s) => s,
        None => "{}".to_string(),
    };

    let query: Value = match serde_json::from_str(&query_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid query JSON: {}", e));
            return ptr::null_mut();
        }
    };

    let options: Value = match serde_json::from_str(&options_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid options JSON: {}", e));
            return ptr::null_mut();
        }
    };

    // Parse options
    use ironbase_core::find_options::FindOptions;
    let mut find_options = FindOptions::new();

    if let Some(proj) = options.get("projection").and_then(|v| v.as_object()) {
        let mut projection_map = HashMap::new();
        for (k, v) in proj {
            if let Some(n) = v.as_i64() {
                projection_map.insert(k.clone(), n as i32);
            }
        }
        find_options.projection = Some(projection_map);
    }

    if let Some(sort) = options.get("sort").and_then(|v| v.as_array()) {
        let mut sort_vec = Vec::new();
        for item in sort {
            if let Some(arr) = item.as_array() {
                if arr.len() == 2 {
                    if let (Some(field), Some(dir)) = (arr[0].as_str(), arr[1].as_i64()) {
                        sort_vec.push((field.to_string(), dir as i32));
                    }
                }
            }
        }
        if !sort_vec.is_empty() {
            find_options.sort = Some(sort_vec);
        }
    }

    find_options.limit = options
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    find_options.skip = options
        .get("skip")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    match coll.inner.find_with_options(&query, find_options) {
        Ok(docs) => match serde_json::to_string(&docs) {
            Ok(json) => string_to_c_str(&json),
            Err(e) => {
                set_last_error(&format!("Failed to serialize results: {}", e));
                ptr::null_mut()
            }
        },
        Err(e) => {
            set_error(&e);
            ptr::null_mut()
        }
    }
}

// ============== UPDATE ==============

/// Update one document
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `update_json`: Update operations as JSON string
/// - `out_matched`: Pointer to receive matched count
/// - `out_modified`: Pointer to receive modified count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_update_one(
    handle: CollHandle,
    query_json: *const c_char,
    update_json: *const c_char,
    out_matched: *mut u64,
    out_modified: *mut u64,
) -> i32 {
    clear_last_error();

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

    let update_str = match c_str_to_string(update_json) {
        Some(s) => s,
        None => {
            set_last_error("Update JSON is null or invalid UTF-8");
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

    let update: Value = match serde_json::from_str(&update_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid update JSON: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    match coll.db.update_one(&coll.name, &query, &update) {
        Ok((matched, modified)) => {
            if !out_matched.is_null() {
                unsafe {
                    *out_matched = matched;
                }
            }
            if !out_modified.is_null() {
                unsafe {
                    *out_modified = modified;
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Update many documents
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `update_json`: Update operations as JSON string
/// - `out_matched`: Pointer to receive matched count
/// - `out_modified`: Pointer to receive modified count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_update_many(
    handle: CollHandle,
    query_json: *const c_char,
    update_json: *const c_char,
    out_matched: *mut u64,
    out_modified: *mut u64,
) -> i32 {
    clear_last_error();

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

    let update_str = match c_str_to_string(update_json) {
        Some(s) => s,
        None => {
            set_last_error("Update JSON is null or invalid UTF-8");
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

    let update: Value = match serde_json::from_str(&update_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid update JSON: {}", e));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    match coll.db.update_many(&coll.name, &query, &update) {
        Ok((matched, modified)) => {
            if !out_matched.is_null() {
                unsafe {
                    *out_matched = matched;
                }
            }
            if !out_modified.is_null() {
                unsafe {
                    *out_modified = modified;
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

// ============== DELETE ==============

/// Delete one document
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `out_deleted`: Pointer to receive deleted count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_delete_one(
    handle: CollHandle,
    query_json: *const c_char,
    out_deleted: *mut u64,
) -> i32 {
    clear_last_error();

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

    match coll.db.delete_one(&coll.name, &query) {
        Ok(count) => {
            if !out_deleted.is_null() {
                unsafe {
                    *out_deleted = count;
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

/// Delete many documents
///
/// # Parameters
/// - `handle`: The collection handle
/// - `query_json`: Query filter as JSON string
/// - `out_deleted`: Pointer to receive deleted count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_delete_many(
    handle: CollHandle,
    query_json: *const c_char,
    out_deleted: *mut u64,
) -> i32 {
    clear_last_error();

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

    match coll.db.delete_many(&coll.name, &query) {
        Ok(count) => {
            if !out_deleted.is_null() {
                unsafe {
                    *out_deleted = count;
                }
            }
            IronBaseErrorCode::Success as i32
        }
        Err(e) => set_error(&e) as i32,
    }
}

// ============== HELPERS ==============

/// Convert JSON Value to HashMap<String, Value>
fn json_to_hashmap(value: &Value) -> Option<HashMap<String, Value>> {
    match value {
        Value::Object(map) => {
            let mut result = HashMap::new();
            for (k, v) in map {
                result.insert(k.clone(), v.clone());
            }
            Some(result)
        }
        _ => None,
    }
}

/// Convert DocumentId to JSON string
fn document_id_to_json(id: &DocumentId) -> String {
    match id {
        DocumentId::Int(i) => i.to_string(),
        DocumentId::String(s) => format!("\"{}\"", s),
        DocumentId::ObjectId(s) => format!("\"{}\"", s),
    }
}
