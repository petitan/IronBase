//! Transaction FFI functions
//!
//! ACD (Atomicity, Consistency, Durability) transaction support

use std::collections::HashMap;
use std::os::raw::c_char;

use serde_json::Value;

use crate::error::{
    c_str_to_string, clear_last_error, set_error, set_last_error, string_to_c_str,
    IronBaseErrorCode,
};
use crate::handles::{validate_db_handle, DbHandle};

/// Begin a new transaction
///
/// # Parameters
/// - `handle`: The database handle
/// - `out_tx_id`: Pointer to receive the transaction ID
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Usage
/// ```c
/// uint64_t tx_id;
/// ironbase_begin_transaction(db, &tx_id);
/// // ... perform operations with tx_id ...
/// ironbase_commit(db, tx_id);
/// // or ironbase_rollback(db, tx_id);
/// ```
#[no_mangle]
pub extern "C" fn ironbase_begin_transaction(handle: DbHandle, out_tx_id: *mut u64) -> i32 {
    clear_last_error();

    if out_tx_id.is_null() {
        set_last_error("out_tx_id is null");
        return IronBaseErrorCode::NullPointer as i32;
    }

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let tx_id = db.inner.begin_transaction();
    unsafe {
        *out_tx_id = tx_id;
    }

    IronBaseErrorCode::Success as i32
}

/// Commit a transaction
///
/// Applies all buffered operations atomically.
///
/// # Parameters
/// - `handle`: The database handle
/// - `tx_id`: The transaction ID from `ironbase_begin_transaction()`
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_commit(handle: DbHandle, tx_id: u64) -> i32 {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    match db.inner.commit_transaction(tx_id) {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Rollback a transaction
///
/// Discards all buffered operations.
///
/// # Parameters
/// - `handle`: The database handle
/// - `tx_id`: The transaction ID from `ironbase_begin_transaction()`
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_rollback(handle: DbHandle, tx_id: u64) -> i32 {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    match db.inner.rollback_transaction(tx_id) {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Insert one document within a transaction
///
/// # Parameters
/// - `handle`: The database handle
/// - `collection_name`: Collection name
/// - `document_json`: Document as JSON string
/// - `tx_id`: Transaction ID
/// - `out_id`: Pointer to receive the inserted ID as JSON string
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_insert_one_tx(
    handle: DbHandle,
    collection_name: *const c_char,
    document_json: *const c_char,
    tx_id: u64,
    out_id: *mut *mut c_char,
) -> i32 {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let coll_name = match c_str_to_string(collection_name) {
        Some(s) => s,
        None => {
            set_last_error("Collection name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
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

    let doc_map = match json_to_hashmap(&doc) {
        Some(m) => m,
        None => {
            set_last_error("Document must be a JSON object");
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    match db.inner.insert_one_tx(&coll_name, doc_map, tx_id) {
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

/// Update one document within a transaction
///
/// # Parameters
/// - `handle`: The database handle
/// - `collection_name`: Collection name
/// - `query_json`: Query filter as JSON string
/// - `new_doc_json`: New document content (full replacement)
/// - `tx_id`: Transaction ID
/// - `out_matched`: Pointer to receive matched count
/// - `out_modified`: Pointer to receive modified count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_update_one_tx(
    handle: DbHandle,
    collection_name: *const c_char,
    query_json: *const c_char,
    new_doc_json: *const c_char,
    tx_id: u64,
    out_matched: *mut u64,
    out_modified: *mut u64,
) -> i32 {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let coll_name = match c_str_to_string(collection_name) {
        Some(s) => s,
        None => {
            set_last_error("Collection name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let query_str = match c_str_to_string(query_json) {
        Some(s) => s,
        None => {
            set_last_error("Query JSON is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let new_doc_str = match c_str_to_string(new_doc_json) {
        Some(s) => s,
        None => {
            set_last_error("New document JSON is null or invalid UTF-8");
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

    let new_doc: Value = match serde_json::from_str(&new_doc_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid new document JSON: {}", e));
            return IronBaseErrorCode::SerializationError as i32;
        }
    };

    match db.inner.update_one_tx(&coll_name, &query, new_doc, tx_id) {
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

/// Delete one document within a transaction
///
/// # Parameters
/// - `handle`: The database handle
/// - `collection_name`: Collection name
/// - `query_json`: Query filter as JSON string
/// - `tx_id`: Transaction ID
/// - `out_deleted`: Pointer to receive deleted count
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_delete_one_tx(
    handle: DbHandle,
    collection_name: *const c_char,
    query_json: *const c_char,
    tx_id: u64,
    out_deleted: *mut u64,
) -> i32 {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return IronBaseErrorCode::InvalidHandle as i32;
        }
    };

    let coll_name = match c_str_to_string(collection_name) {
        Some(s) => s,
        None => {
            set_last_error("Collection name is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
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

    match db.inner.delete_one_tx(&coll_name, &query, tx_id) {
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

use ironbase_core::DocumentId;

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

fn document_id_to_json(id: &DocumentId) -> String {
    match id {
        DocumentId::Int(i) => i.to_string(),
        DocumentId::String(s) => format!("\"{}\"", s),
        DocumentId::ObjectId(s) => format!("\"{}\"", s),
    }
}
