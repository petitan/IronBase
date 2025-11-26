//! Aggregation pipeline FFI functions

use std::os::raw::c_char;
use std::ptr;

use crate::handles::{CollHandle, validate_coll_handle};
use crate::error::{set_last_error, set_error, clear_last_error, c_str_to_string, string_to_c_str};

/// Execute an aggregation pipeline
///
/// # Parameters
/// - `handle`: The collection handle
/// - `pipeline_json`: Aggregation pipeline as JSON array
///
/// # Returns
/// - JSON array of results (caller must free with `ironbase_free_string()`)
/// - Null on error
///
/// # Example Pipeline
/// ```json
/// [
///   {"$match": {"age": {"$gte": 18}}},
///   {"$group": {"_id": "$city", "count": {"$sum": 1}}},
///   {"$sort": {"count": -1}},
///   {"$limit": 10}
/// ]
/// ```
///
/// # Supported Stages
/// - `$match` - Filter documents
/// - `$group` - Group and aggregate
/// - `$project` - Reshape documents
/// - `$sort` - Sort results
/// - `$limit` - Limit result count
/// - `$skip` - Skip documents
///
/// # Supported Accumulators (in $group)
/// - `$sum` - Sum values
/// - `$avg` - Average values
/// - `$min` - Minimum value
/// - `$max` - Maximum value
/// - `$first` - First value
/// - `$last` - Last value
/// - `$count` - Count documents
#[no_mangle]
pub extern "C" fn ironbase_aggregate(
    handle: CollHandle,
    pipeline_json: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let pipeline_str = match c_str_to_string(pipeline_json) {
        Some(s) => s,
        None => {
            set_last_error("Pipeline JSON is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let pipeline: serde_json::Value = match serde_json::from_str(&pipeline_str) {
        Ok(v) => v,
        Err(e) => {
            set_last_error(&format!("Invalid pipeline JSON: {}", e));
            return ptr::null_mut();
        }
    };

    // Validate that pipeline is an array
    if !pipeline.is_array() {
        set_last_error("Pipeline must be a JSON array");
        return ptr::null_mut();
    }

    match coll.inner.aggregate(&pipeline) {
        Ok(results) => {
            match serde_json::to_string(&results) {
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
