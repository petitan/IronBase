//! Schema validation FFI functions
//!
//! Provides JSON schema validation for collections

use std::os::raw::c_char;

use crate::handles::{DbHandle, validate_db_handle};
use crate::error::{IronBaseErrorCode, set_last_error, set_error, clear_last_error, c_str_to_string};

/// Set or clear JSON schema for a collection
///
/// # Parameters
/// - `handle`: The database handle
/// - `collection_name`: Collection name (UTF-8 null-terminated string)
/// - `schema_json`: JSON schema definition (null to clear schema)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
///
/// # Example Schema
/// ```json
/// {
///   "type": "object",
///   "properties": {
///     "name": {"type": "string"},
///     "age": {"type": "integer", "minimum": 0}
///   },
///   "required": ["name"]
/// }
/// ```
///
/// # Notes
/// - Documents that don't match the schema will be rejected on insert/update
/// - Pass null for schema_json to disable schema validation
#[no_mangle]
pub extern "C" fn ironbase_set_collection_schema(
    handle: DbHandle,
    collection_name: *const c_char,
    schema_json: *const c_char,
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

    // Parse schema JSON (null means clear schema)
    let schema = if schema_json.is_null() {
        None
    } else {
        match c_str_to_string(schema_json) {
            Some(s) => {
                match serde_json::from_str(&s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        set_last_error(&format!("Invalid schema JSON: {}", e));
                        return IronBaseErrorCode::SerializationError as i32;
                    }
                }
            }
            None => {
                set_last_error("Schema JSON is invalid UTF-8");
                return IronBaseErrorCode::NullPointer as i32;
            }
        }
    };

    match db.inner.set_collection_schema(&coll_name, schema) {
        Ok(_) => IronBaseErrorCode::Success as i32,
        Err(e) => set_error(&e) as i32,
    }
}

/// Get JSON schema for a collection
///
/// # Parameters
/// - `handle`: The database handle
/// - `collection_name`: Collection name (UTF-8 null-terminated string)
///
/// # Returns
/// - Pointer to JSON string if schema exists (caller must free with `ironbase_free_string`)
/// - NULL if no schema is set
/// - NULL on error (check `ironbase_get_last_error`)
///
/// # Example
/// ```c
/// char* schema = ironbase_get_collection_schema(db, "users");
/// if (schema != NULL) {
///     printf("Schema: %s\n", schema);
///     ironbase_free_string(schema);
/// }
/// ```
#[no_mangle]
pub extern "C" fn ironbase_get_collection_schema(
    handle: DbHandle,
    collection_name: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let db = match validate_db_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid database handle");
            return std::ptr::null_mut();
        }
    };

    let coll_name = match c_str_to_string(collection_name) {
        Some(s) => s,
        None => {
            set_last_error("Collection name is null or invalid UTF-8");
            return std::ptr::null_mut();
        }
    };

    // Get collection
    let collection = match db.inner.collection(&coll_name) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(&format!("Failed to get collection: {}", e));
            return std::ptr::null_mut();
        }
    };

    // Get schema
    match collection.get_schema() {
        Some(schema) => {
            // Convert schema to JSON string
            match serde_json::to_string(&schema) {
                Ok(json_str) => {
                    match std::ffi::CString::new(json_str) {
                        Ok(c_str) => c_str.into_raw(),
                        Err(e) => {
                            set_last_error(&format!("Failed to create C string: {}", e));
                            std::ptr::null_mut()
                        }
                    }
                }
                Err(e) => {
                    set_last_error(&format!("Failed to serialize schema: {}", e));
                    std::ptr::null_mut()
                }
            }
        }
        None => std::ptr::null_mut(), // No schema set - return NULL (not an error)
    }
}
