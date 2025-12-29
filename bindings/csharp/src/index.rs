//! Index management FFI functions

use std::os::raw::c_char;
use std::ptr;

use crate::error::{
    c_str_to_string, clear_last_error, set_error, set_last_error, string_to_c_str,
    IronBaseErrorCode,
};
use crate::handles::{validate_coll_handle, CollHandle};

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

    // sparse=false for backwards compatibility
    match coll.inner.create_index(field_str, unique != 0, false) {
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

    // sparse=false for backwards compatibility
    match coll.inner.create_compound_index(fields, unique != 0, false) {
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
pub extern "C" fn ironbase_drop_index(handle: CollHandle, index_name: *const c_char) -> i32 {
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

    let indexes = match coll.inner.list_indexes() {
        Ok(idx) => idx,
        Err(e) => {
            set_last_error(&format!("Failed to list indexes: {}", e));
            return ptr::null_mut();
        }
    };
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
pub extern "C" fn ironbase_explain(handle: CollHandle, query_json: *const c_char) -> *mut c_char {
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
        Ok(plan) => match serde_json::to_string_pretty(&plan) {
            Ok(json) => string_to_c_str(&json),
            Err(e) => {
                set_last_error(&format!("Failed to serialize plan: {}", e));
                ptr::null_mut()
            }
        },
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

// ============================================================================
// FUZZY SEARCH FFI FUNCTIONS
// ============================================================================

/// Create a fuzzy text index for approximate string matching
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field name to index (UTF-8 null-terminated string)
/// - `algorithm`: Algorithm name ("jaro_winkler", "levenshtein", "damerau_levenshtein")
/// - `threshold`: Similarity threshold (0.0-1.0)
/// - `out_name`: Pointer to receive the index name (optional)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_create_fuzzy_index(
    handle: CollHandle,
    field: *const c_char,
    algorithm: *const c_char,
    threshold: f64,
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

    let algo_str = match c_str_to_string(algorithm) {
        Some(s) => s,
        None => {
            set_last_error("Algorithm is null or invalid UTF-8");
            return IronBaseErrorCode::NullPointer as i32;
        }
    };

    let algo = match algo_str.to_lowercase().as_str() {
        "jaro_winkler" | "jarowinkler" => ironbase_core::index::FuzzyAlgorithm::JaroWinkler,
        "levenshtein" => ironbase_core::index::FuzzyAlgorithm::Levenshtein,
        "damerau_levenshtein" | "dameraulevenshtein" => {
            ironbase_core::index::FuzzyAlgorithm::DamerauLevenshtein
        }
        _ => {
            set_last_error(&format!(
                "Invalid algorithm '{}'. Use 'jaro_winkler', 'levenshtein', or 'damerau_levenshtein'",
                algo_str
            ));
            return IronBaseErrorCode::InvalidQuery as i32;
        }
    };

    match coll.inner.create_fuzzy_index(field_str, algo, threshold) {
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

/// Search for documents using fuzzy text matching
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field name to search
/// - `query`: Search term
/// - `threshold`: Similarity threshold (0.0-1.0), or negative to use index default
/// - `algorithm`: Algorithm override (or null to use index default)
///
/// # Returns
/// - JSON array of [{"doc": {...}, "score": 0.95}, ...] on success
/// - NULL on failure (check ironbase_get_last_error)
#[no_mangle]
pub extern "C" fn ironbase_fuzzy_search(
    handle: CollHandle,
    field: *const c_char,
    query: *const c_char,
    threshold: f64,
    algorithm: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let field_str = match c_str_to_string(field) {
        Some(s) => s,
        None => {
            set_last_error("Field is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let query_str = match c_str_to_string(query) {
        Some(s) => s,
        None => {
            set_last_error("Query is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let threshold_opt = if threshold < 0.0 {
        None
    } else {
        Some(threshold)
    };

    let algo_opt = if algorithm.is_null() {
        None
    } else {
        match c_str_to_string(algorithm) {
            Some(s) => match s.to_lowercase().as_str() {
                "jaro_winkler" | "jarowinkler" => {
                    Some(ironbase_core::index::FuzzyAlgorithm::JaroWinkler)
                }
                "levenshtein" => Some(ironbase_core::index::FuzzyAlgorithm::Levenshtein),
                "damerau_levenshtein" | "dameraulevenshtein" => {
                    Some(ironbase_core::index::FuzzyAlgorithm::DamerauLevenshtein)
                }
                _ => {
                    set_last_error(&format!(
                        "Invalid algorithm '{}'. Use 'jaro_winkler', 'levenshtein', or 'damerau_levenshtein'",
                        s
                    ));
                    return ptr::null_mut();
                }
            },
            None => None,
        }
    };

    match coll
        .inner
        .fuzzy_search(&field_str, &query_str, threshold_opt, algo_opt)
    {
        Ok(results) => {
            // Convert Vec<(Value, f64)> to JSON array of objects
            let json_results: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(doc, score)| {
                    serde_json::json!({
                        "doc": doc,
                        "score": score
                    })
                })
                .collect();

            match serde_json::to_string(&json_results) {
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

// ============================================================================
// FULL-TEXT SEARCH FFI FUNCTIONS
// ============================================================================

/// Create a full-text search index with TF-IDF scoring
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field name to index
/// - `language`: Language for stemming ("english", "hungarian", "german", "none")
/// - `min_word_length`: Minimum word length (0 for default)
/// - `accent_folding`: Normalize accents (1 = true, 0 = false, -1 = default)
/// - `out_name`: Pointer to receive the index name (optional)
///
/// # Returns
/// - `IronBaseErrorCode::Success` (0) on success
/// - Error code on failure
#[no_mangle]
pub extern "C" fn ironbase_create_fulltext_index(
    handle: CollHandle,
    field: *const c_char,
    language: *const c_char,
    min_word_length: i32,
    accent_folding: i32,
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

    let lang_str = match c_str_to_string(language) {
        Some(s) => s,
        None => "english".to_string(),
    };

    let min_len = if min_word_length > 0 {
        Some(min_word_length as usize)
    } else {
        None
    };

    let accent = if accent_folding < 0 {
        None
    } else {
        Some(accent_folding != 0)
    };

    match coll
        .inner
        .create_fulltext_index(field_str, &lang_str, min_len, accent)
    {
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

/// Search documents using full-text search with TF-IDF scoring
///
/// # Parameters
/// - `handle`: The collection handle
/// - `field`: Field to search
/// - `query`: Search query (words separated by spaces)
/// - `limit`: Maximum results (0 for default)
/// - `skip`: Number of results to skip
/// - `min_score`: Minimum score filter (negative for no filter)
/// - `projection_json`: JSON object for projection (null for all fields)
///
/// # Returns
/// - JSON array of [{"doc": {...}, "score": 0.95, "tokens": ["word1", "word2"]}, ...] on success
/// - NULL on failure
#[no_mangle]
pub extern "C" fn ironbase_fulltext_search(
    handle: CollHandle,
    field: *const c_char,
    query: *const c_char,
    limit: i32,
    skip: i32,
    min_score: f64,
    projection_json: *const c_char,
) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    let field_str = match c_str_to_string(field) {
        Some(s) => s,
        None => {
            set_last_error("Field is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let query_str = match c_str_to_string(query) {
        Some(s) => s,
        None => {
            set_last_error("Query is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };

    let limit_opt = if limit > 0 {
        Some(limit as usize)
    } else {
        None
    };
    let skip_opt = if skip > 0 { Some(skip as usize) } else { None };
    let min_score_opt = if min_score >= 0.0 {
        Some(min_score)
    } else {
        None
    };

    let projection_opt: Option<std::collections::HashMap<String, i32>> =
        if projection_json.is_null() {
            None
        } else {
            match c_str_to_string(projection_json) {
                Some(s) => match serde_json::from_str(&s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        set_last_error(&format!("Invalid projection JSON: {}", e));
                        return ptr::null_mut();
                    }
                },
                None => None,
            }
        };

    match coll.inner.fulltext_search(
        &field_str,
        &query_str,
        limit_opt,
        skip_opt,
        min_score_opt,
        projection_opt,
    ) {
        Ok(results) => {
            // Convert Vec<(Value, f64, Vec<String>)> to JSON
            let json_results: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(doc, score, tokens)| {
                    serde_json::json!({
                        "doc": doc,
                        "score": score,
                        "tokens": tokens
                    })
                })
                .collect();

            match serde_json::to_string(&json_results) {
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

/// List all fulltext indexes for a collection
///
/// # Parameters
/// - `handle`: The collection handle
///
/// # Returns
/// - JSON array of index metadata on success
/// - NULL on failure
#[no_mangle]
pub extern "C" fn ironbase_list_fulltext_indexes(handle: CollHandle) -> *mut c_char {
    clear_last_error();

    let coll = match validate_coll_handle(handle) {
        Some(h) => h,
        None => {
            set_last_error("Invalid collection handle");
            return ptr::null_mut();
        }
    };

    match coll.inner.list_fulltext_indexes() {
        Ok(indexes) => {
            let json_indexes: Vec<serde_json::Value> = indexes
                .into_iter()
                .map(|idx| {
                    serde_json::json!({
                        "name": idx.name,
                        "field": idx.field,
                        "language": format!("{:?}", idx.language),
                        "min_word_length": idx.min_word_length,
                        "accent_folding": idx.accent_folding,
                        "num_documents": idx.num_documents,
                        "num_tokens": idx.num_tokens
                    })
                })
                .collect();

            match serde_json::to_string(&json_indexes) {
                Ok(json) => string_to_c_str(&json),
                Err(e) => {
                    set_last_error(&format!("Failed to serialize indexes: {}", e));
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
