//! Database function registrations for Rhai scripts.
//!
//! Provides database operations accessible from scripts:
//! - CRUD operations (find, insert, update, delete)
//! - Aggregation
//! - Index management (B+ tree, fuzzy, fulltext, vector)
//! - Fuzzy, fulltext, and vector similarity search
//! - RAG operations (semantic search with auto-embedding)
//!
//! All functions include safety limits to prevent OOM attacks.

use crate::adapter::{FindOptions as AdapterFindOptions, FulltextSearchOptions, IronBaseAdapter};
use crate::chunking::{chunk_content, ChunkMode, ChunkOptions};
use crate::embedding::EmbeddingManager;
use crate::tools::defaults::{
    DEFAULT_EMBEDDING_FIELD, DEFAULT_EMBEDDING_PROVIDER, DEFAULT_RRF_K, DEFAULT_TEXT_FIELD,
    MAX_INTERNAL_LIMIT,
};
use crate::tools::fusion::{
    id_to_string as fusion_id_to_string, merge_adjacent_chunks, mmr_reorder, rerank_results,
    FusedResult,
};
use rhai::{Dynamic, Engine, Map};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::conversion::{dynamic_to_json, json_to_dynamic, map_to_json};
use super::limits::{ScriptLimits, ABSOLUTE_MAX_FIND_DOCUMENTS};

fn projection_from_dynamic(value: &Dynamic) -> Result<Option<HashMap<String, i32>>, String> {
    let proj_json = dynamic_to_json(value);
    if proj_json.is_null() {
        return Ok(None);
    }
    let obj = proj_json.as_object().ok_or_else(|| {
        "projection must be an object like {\"field\": 1} or {\"field\": 0}".to_string()
    })?;
    let mut projection = HashMap::new();
    for (key, val) in obj {
        let entry = if let Some(i) = val.as_i64() {
            if i != 0 && i != 1 {
                return Err(format!(
                    "Invalid projection value for '{}': expected 0 or 1, got {}",
                    key, i
                ));
            }
            i as i32
        } else if let Some(f) = val.as_f64() {
            if f == 0.0 {
                0
            } else if f == 1.0 {
                1
            } else {
                return Err(format!(
                    "Invalid projection value for '{}': expected 0 or 1, got {}",
                    key, f
                ));
            }
        } else {
            return Err(format!(
                "Invalid projection value for '{}': expected 0 or 1, got {:?}",
                key, val
            ));
        };
        projection.insert(key.clone(), entry);
    }
    if projection.is_empty() {
        Ok(None)
    } else {
        Ok(Some(projection))
    }
}

// ============================================================
// Option Parsing Helpers (reduce code duplication)
// ============================================================

/// Extract string option from Map, avoiding unnecessary clones where possible
fn get_string_option(options: &Map, key: &str) -> Option<String> {
    options.get(key).and_then(|v| {
        // Try to get immutable string reference first
        if let Some(s) = v.read_lock::<rhai::ImmutableString>() {
            return Some(s.to_string());
        }
        // Fallback to clone + cast
        v.clone().try_cast::<String>()
    })
}

/// Extract string option with default value
fn get_string_option_or(options: &Map, key: &str, default: &str) -> String {
    get_string_option(options, key).unwrap_or_else(|| default.to_string())
}

/// Extract integer option from Map
fn get_int_option(options: &Map, key: &str) -> Option<i64> {
    options.get(key).and_then(|v| v.as_int().ok())
}

/// Extract integer option with default value
fn get_int_option_or(options: &Map, key: &str, default: i64) -> i64 {
    get_int_option(options, key).unwrap_or(default)
}

/// Extract float option from Map
fn get_float_option(options: &Map, key: &str) -> Option<f64> {
    options.get(key).and_then(|v| v.as_float().ok())
}

/// Extract float option with default value
fn get_float_option_or(options: &Map, key: &str, default: f64) -> f64 {
    get_float_option(options, key).unwrap_or(default)
}

/// Extract bool option from Map
fn get_bool_option(options: &Map, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| v.as_bool().ok())
}

/// Extract bool option with default value
fn get_bool_option_or(options: &Map, key: &str, default: bool) -> bool {
    get_bool_option(options, key).unwrap_or(default)
}

/// Extract JSON Value option from Map (returns None if key missing or value is null)
fn get_json_option(options: &Map, key: &str) -> Option<serde_json::Value> {
    options.get(key).and_then(|v| {
        let json = dynamic_to_json(v);
        if json.is_null() {
            None
        } else {
            Some(json)
        }
    })
}

/// Extract string array option from Map
fn get_string_array_option(options: &Map, key: &str) -> Option<Vec<String>> {
    options.get(key).and_then(|v| {
        v.clone().try_cast::<rhai::Array>().map(|arr| {
            arr.into_iter()
                .filter_map(|item| {
                    // Clone for read_lock to avoid borrow conflict with try_cast
                    if let Some(s) = item.clone().read_lock::<rhai::ImmutableString>() {
                        Some(s.to_string())
                    } else {
                        item.try_cast::<String>()
                    }
                })
                .collect()
        })
    })
}

/// Resolve weights from search_mode preset or explicit overrides
///
/// Priority: explicit weights > search_mode preset > balanced default
fn resolve_search_mode_weights(opts: &Map, search_mode: Option<&str>) -> (f64, f64) {
    let explicit_vw = get_float_option(opts, "vector_weight");
    let explicit_fw = get_float_option(opts, "fulltext_weight");
    if explicit_vw.is_some() || explicit_fw.is_some() {
        return (explicit_vw.unwrap_or(0.5), explicit_fw.unwrap_or(0.5));
    }
    match search_mode {
        Some("semantic") => (0.8, 0.2),
        Some("keyword") => (0.2, 0.8),
        _ => (0.5, 0.5),
    }
}

/// Register all database functions into a Rhai engine.
///
/// # Functions Registered
///
/// ## Read Operations
/// - `db_find(collection, query)` - Find documents (returns documents + count)
/// - `db_find(collection, query, options)` - Find with options (returns documents + count + total)
/// - `db_find_one(collection, query)` - Find single document
/// - `db_find_one_result(collection, query)` - Find with explicit result type
/// - `db_count(collection, query)` - Count documents
/// - `db_aggregate(collection, pipeline)` - Run aggregation
/// - `db_distinct(collection, field, query)` - Get distinct values
///
/// ## Write Operations (blocked for _system.* collections)
/// - `db_insert_one(collection, doc)` - Insert single document
/// - `db_insert_many(collection, docs)` - Insert multiple documents
/// - `db_update_one(collection, filter, update)` - Update single document
/// - `db_update_many(collection, filter, update)` - Update multiple documents
/// - `db_delete_one(collection, filter)` - Delete single document
/// - `db_delete_many(collection, filter)` - Delete multiple documents
///
/// ## Index Operations
/// - `db_create_index(collection, field, unique)` - Create index
/// - `db_create_compound_index(collection, fields, unique)` - Create compound index
/// - `db_list_indexes(collection)` - List indexes
/// - `db_drop_index(collection, index_name)` - Drop index
/// - `db_explain(collection, query)` - Explain query plan
///
/// ## Search Operations
/// - `db_create_fuzzy_index(collection, field, algorithm, threshold)` - Create fuzzy index
/// - `db_fuzzy_search(collection, field, query, threshold)` - Fuzzy search
/// - `db_create_fulltext_index(collection, field, language)` - Create fulltext index
/// - `db_fulltext_search(collection, field, query, options)` - Fulltext search with options
///
/// ## Vector Index Operations
/// - `db_create_vector_index(collection, field, dim, metric)` - Create vector index
/// - `db_create_vector_index(collection, field, dim, metric, options)` - Create with HNSW options
/// - `db_list_vector_indexes(collection)` - List vector indexes
/// - `db_drop_vector_index(collection, index_name)` - Drop vector index
/// - `db_vector_search(collection, field, query_vector, limit)` - Vector similarity search
/// - `db_vector_search_filter(collection, field, query_vector, filter, limit)` - Vector search with filter
///
/// ## RAG Operations (require embedding_manager)
/// - `db_hybrid_search(collection, query)` - Hybrid search (auto-embed, RRF fusion)
/// - `db_hybrid_search(collection, query, options)` - Hybrid search with options
/// - `db_rag_import(collection, content, title)` - Import document with chunking
/// - `db_rag_import(collection, content, options)` - Import with options
/// - `db_rag_create(collection)` - Create RAG collection with defaults
/// - `db_rag_create(collection, options)` - Create RAG collection with options
/// - `db_rag_stats(collection)` - Get RAG collection statistics
///
/// # Arguments
///
/// * `engine` - The Rhai engine to register functions into
/// * `adapter` - Database adapter for executing operations
/// * `embedding_manager` - Optional embedding manager for RAG operations
/// * `limits` - Resource limits for query operations
///
/// # Security
///
/// All write operations to `_system.*` collections are blocked.
/// Query limits are enforced to prevent OOM attacks.
pub fn register_db_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    embedding_manager: Option<Arc<EmbeddingManager>>,
    limits: &ScriptLimits,
) {
    register_read_functions(engine, adapter.clone(), limits);
    register_write_functions(engine, adapter.clone());
    register_index_functions(engine, adapter.clone());
    register_search_functions(engine, adapter.clone(), limits);
    register_vector_functions(engine, adapter.clone(), limits);
    register_rag_functions(engine, adapter, embedding_manager, limits);
}

// ============================================================
// Read Operations
// ============================================================

fn register_read_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    limits: &ScriptLimits,
) {
    let max_find_documents = limits.max_find_documents;
    let default_limit = max_find_documents.min(ABSOLUTE_MAX_FIND_DOCUMENTS);

    // db_find(collection, query) -> #{documents: [...], count: int}
    // SECURITY: Always applies default limit to prevent OOM
    let adapter_find = adapter.clone();
    engine.register_fn("db_find", move |collection: &str, query: Map| -> Dynamic {
        let query_json = map_to_json(&query);

        // SECURITY: Force limit to prevent OOM
        let options = AdapterFindOptions {
            limit: Some(default_limit),
            ..Default::default()
        };

        match adapter_find.find(collection, query_json, options) {
            Ok(result) => {
                let count = result.documents.len() as i64;
                let docs: Vec<Dynamic> = result
                    .documents
                    .into_iter()
                    .map(|d| json_to_dynamic(&d))
                    .collect();
                let mut map = Map::new();
                map.insert("documents".into(), Dynamic::from(docs));
                map.insert("count".into(), Dynamic::from(count));
                Dynamic::from(map)
            }
            Err(e) => Dynamic::from(format!("Error: {}", e)),
        }
    });

    // db_find(collection, query, options) -> #{documents: [...], count: int, total?: int}
    // Options: { limit: int, skip: int, sort: { field: 1|-1 }, projection: { field: 1|0 } }
    let adapter_find_opts = adapter.clone();
    engine.register_fn(
        "db_find",
        move |collection: &str, query: Map, options: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            let mut find_options = AdapterFindOptions::default();

            // Parse limit - ENFORCE maximum
            let requested_limit = options
                .get("limit")
                .and_then(|v| v.as_int().ok())
                .map(|l| l as usize)
                .unwrap_or(default_limit);

            // Enforce script limits and absolute cap
            find_options.limit = Some(
                requested_limit
                    .min(max_find_documents)
                    .min(ABSOLUTE_MAX_FIND_DOCUMENTS),
            );

            // Parse skip
            if let Some(skip_val) = options.get("skip") {
                if let Ok(skip) = skip_val.as_int() {
                    if skip > 0 {
                        find_options.skip = Some(skip as usize);
                    }
                }
            }

            // Parse sort: { field: 1 } or { field: -1 } or { a: 1, b: -1 }
            if let Some(sort_val) = options.get("sort") {
                if let Some(sort_map) = sort_val.clone().try_cast::<Map>() {
                    let mut sort_vec = Vec::new();
                    for (key, val) in sort_map.iter() {
                        if let Ok(dir) = val.as_int() {
                            sort_vec.push((key.to_string(), dir as i32));
                        }
                    }
                    if !sort_vec.is_empty() {
                        find_options.sort = Some(sort_vec);
                    }
                }
            }

            // Parse projection: { field: 1 } or { field: 0 }
            if let Some(proj_val) = options.get("projection") {
                match projection_from_dynamic(proj_val) {
                    Ok(Some(projection)) => {
                        find_options.projection = Some(serde_json::json!(projection));
                    }
                    Ok(None) => {}
                    Err(message) => return Dynamic::from(format!("Error: {}", message)),
                }
            }

            // Parse include_total
            if let Some(include_val) = options.get("include_total") {
                if let Ok(include) = include_val.as_bool() {
                    find_options.include_total = include;
                }
            }

            match adapter_find_opts.find(collection, query_json, find_options) {
                Ok(result) => {
                    let count = result.documents.len() as i64;
                    let docs: Vec<Dynamic> = result
                        .documents
                        .into_iter()
                        .map(|d| json_to_dynamic(&d))
                        .collect();
                    let mut map = Map::new();
                    map.insert("documents".into(), Dynamic::from(docs));
                    map.insert("count".into(), Dynamic::from(count));
                    if let Some(total) = result.total {
                        map.insert("total".into(), Dynamic::from(total as i64));
                    }
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_find_one(collection, query) -> document or ()
    let adapter_find_one = adapter.clone();
    engine.register_fn(
        "db_find_one",
        move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_find_one.find_one(collection, query_json) {
                Ok(Some(doc)) => json_to_dynamic(&doc),
                Ok(None) => Dynamic::UNIT,
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_find_one_result(collection, query) -> #{found: bool, doc: ..., error: ...}
    let adapter_find_one_result = adapter.clone();
    engine.register_fn(
        "db_find_one_result",
        move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            let mut result_map = Map::new();
            match adapter_find_one_result.find_one(collection, query_json) {
                Ok(Some(doc)) => {
                    result_map.insert("found".into(), Dynamic::from(true));
                    result_map.insert("doc".into(), json_to_dynamic(&doc));
                    result_map.insert("error".into(), Dynamic::UNIT);
                }
                Ok(None) => {
                    result_map.insert("found".into(), Dynamic::from(false));
                    result_map.insert("doc".into(), Dynamic::UNIT);
                    result_map.insert("error".into(), Dynamic::UNIT);
                }
                Err(e) => {
                    result_map.insert("found".into(), Dynamic::from(false));
                    result_map.insert("doc".into(), Dynamic::UNIT);
                    result_map.insert("error".into(), Dynamic::from(e.to_string()));
                }
            }
            Dynamic::from(result_map)
        },
    );

    // db_count(collection, query) -> count
    let adapter_count = adapter.clone();
    engine.register_fn("db_count", move |collection: &str, query: Map| -> Dynamic {
        let query_json = map_to_json(&query);
        match adapter_count.count_documents(collection, query_json) {
            Ok(count) => Dynamic::from(count as i64),
            Err(e) => Dynamic::from(format!("Error: {}", e)),
        }
    });

    // db_aggregate(collection, pipeline) -> array of documents
    // SECURITY: Limit output to MAX_AGGREGATE_DOCUMENTS to prevent OOM
    let adapter_agg = adapter.clone();
    engine.register_fn(
        "db_aggregate",
        move |collection: &str, pipeline: rhai::Array| -> Dynamic {
            let pipeline_vec: Vec<serde_json::Value> =
                pipeline.iter().map(dynamic_to_json).collect();
            match adapter_agg.aggregate(collection, pipeline_vec) {
                Ok(docs) => {
                    // OOM protection: limit output documents
                    let truncated = docs.len() > MAX_AGGREGATE_DOCUMENTS;
                    let result: Vec<Dynamic> = docs
                        .into_iter()
                        .take(MAX_AGGREGATE_DOCUMENTS)
                        .map(|d| json_to_dynamic(&d))
                        .collect();
                    if truncated {
                        tracing::warn!(
                            "db_aggregate result truncated to {} documents (OOM protection)",
                            MAX_AGGREGATE_DOCUMENTS
                        );
                    }
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_distinct(collection, field, query) -> array of unique values
    // SECURITY: Limit output to MAX_DISTINCT_VALUES to prevent OOM
    let adapter_dist = adapter;
    engine.register_fn(
        "db_distinct",
        move |collection: &str, field: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_dist.distinct(collection, field, query_json) {
                Ok(values) => {
                    // OOM protection: limit unique values
                    let truncated = values.len() > MAX_DISTINCT_VALUES;
                    let result: Vec<Dynamic> = values
                        .into_iter()
                        .take(MAX_DISTINCT_VALUES)
                        .map(|v| json_to_dynamic(&v))
                        .collect();
                    if truncated {
                        tracing::warn!(
                            "db_distinct result truncated to {} values (OOM protection)",
                            MAX_DISTINCT_VALUES
                        );
                    }
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );
}

// ============================================================
// Write Operations (with _system.* protection)
// ============================================================

/// Check if collection is a system collection (write protected).
fn is_system_collection(collection: &str) -> bool {
    collection.starts_with("_system.")
}

/// Error message for system collection write attempt.
const SYSTEM_COLLECTION_ERROR: &str = "Error: Scripts cannot modify system collections (_system.*)";

fn register_write_functions(engine: &mut Engine, adapter: Arc<IronBaseAdapter>) {
    // db_insert_one(collection, document) -> inserted_id
    let adapter_insert = adapter.clone();
    engine.register_fn(
        "db_insert_one",
        move |collection: &str, doc: Map| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let doc_json = map_to_json(&doc);
            match adapter_insert.insert_one(collection, doc_json) {
                Ok(id) => Dynamic::from(id),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_insert_many(collection, documents_array) -> {inserted_count, inserted_ids}
    let adapter_insert_many = adapter.clone();
    engine.register_fn(
        "db_insert_many",
        move |collection: &str, docs: rhai::Array| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let docs_vec: Vec<serde_json::Value> = docs.iter().map(dynamic_to_json).collect();
            match adapter_insert_many.insert_many(collection, docs_vec) {
                Ok(ids) => {
                    let mut map = Map::new();
                    map.insert("inserted_count".into(), Dynamic::from(ids.len() as i64));
                    let id_dynamics: Vec<Dynamic> = ids.into_iter().map(Dynamic::from).collect();
                    map.insert("inserted_ids".into(), Dynamic::from(id_dynamics));
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_update_one(collection, filter, update) -> {matched_count, modified_count}
    let adapter_update_one = adapter.clone();
    engine.register_fn(
        "db_update_one",
        move |collection: &str, filter: Map, update: Map| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let filter_json = map_to_json(&filter);
            let update_json = map_to_json(&update);
            match adapter_update_one.update_one(collection, filter_json, update_json) {
                Ok(result) => {
                    let mut map = Map::new();
                    map.insert(
                        "matched_count".into(),
                        Dynamic::from(result.matched_count as i64),
                    );
                    map.insert(
                        "modified_count".into(),
                        Dynamic::from(result.modified_count as i64),
                    );
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_update_many(collection, filter, update) -> {matched_count, modified_count}
    let adapter_update_many = adapter.clone();
    engine.register_fn(
        "db_update_many",
        move |collection: &str, filter: Map, update: Map| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let filter_json = map_to_json(&filter);
            let update_json = map_to_json(&update);
            match adapter_update_many.update_many(collection, filter_json, update_json) {
                Ok(result) => {
                    let mut map = Map::new();
                    map.insert(
                        "matched_count".into(),
                        Dynamic::from(result.matched_count as i64),
                    );
                    map.insert(
                        "modified_count".into(),
                        Dynamic::from(result.modified_count as i64),
                    );
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_delete_one(collection, filter) -> deleted_count
    let adapter_delete_one = adapter.clone();
    engine.register_fn(
        "db_delete_one",
        move |collection: &str, filter: Map| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let filter_json = map_to_json(&filter);
            match adapter_delete_one.delete_one(collection, filter_json) {
                Ok(count) => Dynamic::from(count as i64),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_delete_many(collection, filter) -> deleted_count
    let adapter_delete_many = adapter;
    engine.register_fn(
        "db_delete_many",
        move |collection: &str, filter: Map| -> Dynamic {
            if is_system_collection(collection) {
                return Dynamic::from(SYSTEM_COLLECTION_ERROR.to_string());
            }
            let filter_json = map_to_json(&filter);
            match adapter_delete_many.delete_many(collection, filter_json) {
                Ok(count) => Dynamic::from(count as i64),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );
}

// ============================================================
// Index Operations
// ============================================================

fn register_index_functions(engine: &mut Engine, adapter: Arc<IronBaseAdapter>) {
    // db_create_index(collection, field, unique) -> index_name
    let adapter_idx = adapter.clone();
    engine.register_fn(
        "db_create_index",
        move |collection: &str, field: &str, unique: bool| -> Dynamic {
            match adapter_idx.create_index(collection, field, unique, false) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_create_compound_index(collection, fields_array, unique) -> index_name
    let adapter_cidx = adapter.clone();
    engine.register_fn(
        "db_create_compound_index",
        move |collection: &str, fields: rhai::Array, unique: bool| -> Dynamic {
            let field_vec: Vec<String> = fields
                .iter()
                .filter_map(|f| f.clone().try_cast::<String>())
                .collect();
            match adapter_cidx.create_compound_index(collection, &field_vec, unique, false) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_list_indexes(collection) -> #{btree_indexes: [...], fulltext_indexes: [...], vector_indexes: [...], summary: {...}}
    let adapter_lidx = adapter.clone();
    engine.register_fn("db_list_indexes", move |collection: &str| -> Dynamic {
        let btree = adapter_lidx.list_indexes(collection).unwrap_or_default();
        let fulltext = adapter_lidx
            .list_fulltext_indexes(collection)
            .unwrap_or_default();
        let vector = adapter_lidx
            .list_vector_indexes(collection)
            .unwrap_or_default();

        let btree_dyn: Vec<Dynamic> = btree.iter().map(|s| Dynamic::from(s.clone())).collect();
        let fulltext_dyn: Vec<Dynamic> = fulltext.iter().map(json_to_dynamic).collect();
        let vector_dyn: Vec<Dynamic> = vector.iter().map(json_to_dynamic).collect();

        let mut summary = Map::new();
        summary.insert("btree_count".into(), Dynamic::from(btree.len() as i64));
        summary.insert(
            "fulltext_count".into(),
            Dynamic::from(fulltext.len() as i64),
        );
        summary.insert("vector_count".into(), Dynamic::from(vector.len() as i64));
        summary.insert(
            "total".into(),
            Dynamic::from((btree.len() + fulltext.len() + vector.len()) as i64),
        );

        let mut result = Map::new();
        result.insert("btree_indexes".into(), Dynamic::from(btree_dyn));
        result.insert("fulltext_indexes".into(), Dynamic::from(fulltext_dyn));
        result.insert("vector_indexes".into(), Dynamic::from(vector_dyn));
        result.insert("summary".into(), Dynamic::from(summary));
        Dynamic::from(result)
    });

    // db_drop_index(collection, index_name) -> bool
    let adapter_didx = adapter.clone();
    engine.register_fn(
        "db_drop_index",
        move |collection: &str, index_name: &str| -> Dynamic {
            match adapter_didx.drop_index(collection, index_name) {
                Ok(()) => Dynamic::from(true),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_explain(collection, query) -> query plan
    let adapter_expl = adapter;
    engine.register_fn(
        "db_explain",
        move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_expl.explain(collection, query_json) {
                Ok(plan) => json_to_dynamic(&plan),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );
}

// ============================================================
// Search Operations (Fuzzy + Fulltext)
// ============================================================

fn register_search_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    limits: &ScriptLimits,
) {
    // db_create_fuzzy_index(collection, field, algorithm, threshold) -> index_name
    let adapter_fzidx = adapter.clone();
    engine.register_fn(
        "db_create_fuzzy_index",
        move |collection: &str, field: &str, algorithm: &str, threshold: f64| -> Dynamic {
            match adapter_fzidx.create_fuzzy_index(collection, field, algorithm, threshold) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_fuzzy_search(collection, field, query, threshold) -> array of {document, score}
    let adapter_fzsrch = adapter.clone();
    engine.register_fn(
        "db_fuzzy_search",
        move |collection: &str, field: &str, query: &str, threshold: f64| -> Dynamic {
            match adapter_fzsrch.fuzzy_search(collection, field, query, Some(threshold), None, None)
            {
                Ok(results) => {
                    let result: Vec<Dynamic> = results
                        .into_iter()
                        .map(|(doc, score)| {
                            let mut map = Map::new();
                            map.insert("document".into(), json_to_dynamic(&doc));
                            map.insert("score".into(), Dynamic::from(score));
                            Dynamic::from(map)
                        })
                        .collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_create_fulltext_index(collection, field, language) -> index_name
    let adapter_ftidx = adapter.clone();
    engine.register_fn(
        "db_create_fulltext_index",
        move |collection: &str, field: &str, language: &str| -> Dynamic {
            match adapter_ftidx.create_fulltext_index(collection, field, language, None, None) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_fulltext_search(collection, field, query, options) -> array of {document, score, matched_tokens, highlights?}
    let max_find_documents = limits.max_find_documents;
    let default_limit = max_find_documents.min(ABSOLUTE_MAX_FIND_DOCUMENTS);
    let adapter_ftsrch = adapter;
    engine.register_fn(
        "db_fulltext_search",
        move |collection: &str, field: &str, query: &str, options: Map| -> Dynamic {
            let mut requested_limit = default_limit;
            let mut skip = None;
            let mut min_score = None;
            let mut projection = None;
            let mut highlight = false;
            let mut highlight_context = None;
            let mut highlight_max_snippets = None;

            if let Some(limit_val) = options.get("limit") {
                if let Ok(limit) = limit_val.as_int() {
                    if limit > 0 {
                        requested_limit = limit as usize;
                    }
                }
            }

            if let Some(skip_val) = options.get("skip") {
                if let Ok(skip_val) = skip_val.as_int() {
                    if skip_val > 0 {
                        skip = Some(skip_val as usize);
                    }
                }
            }

            if let Some(score_val) = options.get("min_score") {
                if let Ok(score) = score_val.as_float() {
                    min_score = Some(score);
                }
            }

            if let Some(proj_val) = options.get("projection") {
                match projection_from_dynamic(proj_val) {
                    Ok(parsed) => projection = parsed,
                    Err(message) => return Dynamic::from(format!("Error: {}", message)),
                }
            }

            // Highlight options
            if let Some(hl_val) = options.get("highlight") {
                if let Ok(hl) = hl_val.as_bool() {
                    highlight = hl;
                }
            }

            if let Some(ctx_val) = options.get("highlight_context") {
                if let Ok(ctx) = ctx_val.as_int() {
                    if (20..=500).contains(&ctx) {
                        highlight_context = Some(ctx as usize);
                    }
                }
            }

            if let Some(snip_val) = options.get("highlight_max_snippets") {
                if let Ok(snip) = snip_val.as_int() {
                    if (1..=10).contains(&snip) {
                        highlight_max_snippets = Some(snip as usize);
                    }
                }
            }

            let effective_limit = requested_limit
                .min(max_find_documents)
                .min(ABSOLUTE_MAX_FIND_DOCUMENTS);
            let options = FulltextSearchOptions {
                limit: Some(effective_limit),
                skip,
                min_score,
                projection,
                filter: None, // Scripting doesn't support filter yet
                and_mode: false,
                highlight,
                highlight_context,
                highlight_max_snippets,
                target_doc_ids: None,
            };

            match adapter_ftsrch.fulltext_search(collection, field, query, options) {
                Ok(results) => {
                    let result: Vec<Dynamic> = results
                        .into_iter()
                        .map(|res| {
                            let mut map = Map::new();
                            map.insert("document".into(), json_to_dynamic(&res.document));
                            map.insert("score".into(), Dynamic::from(res.score));
                            let token_dyn: Vec<Dynamic> =
                                res.matched_tokens.into_iter().map(Dynamic::from).collect();
                            map.insert("matched_tokens".into(), Dynamic::from(token_dyn));
                            // Add highlights if available
                            if let Some(ref highlights) = res.highlights {
                                let hl_dyn: Vec<Dynamic> = highlights
                                    .iter()
                                    .map(|hl| {
                                        let mut hl_map = Map::new();
                                        hl_map.insert(
                                            "field".into(),
                                            Dynamic::from(hl.field.clone()),
                                        );
                                        let snippets: Vec<Dynamic> = hl
                                            .snippets
                                            .iter()
                                            .map(|s| Dynamic::from(s.clone()))
                                            .collect();
                                        hl_map.insert("snippets".into(), Dynamic::from(snippets));
                                        Dynamic::from(hl_map)
                                    })
                                    .collect();
                                map.insert("highlights".into(), Dynamic::from(hl_dyn));
                            }
                            Dynamic::from(map)
                        })
                        .collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );
}

// ============================================================
// Vector Index Operations
// ============================================================

fn register_vector_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    limits: &ScriptLimits,
) {
    // db_create_vector_index(collection, field, dim, metric) -> index_name
    // metric: "cosine" | "euclidean" | "dot_product"
    let adapter_vidx = adapter.clone();
    engine.register_fn(
        "db_create_vector_index",
        move |collection: &str, field: &str, dim: i64, metric: &str| -> Dynamic {
            match adapter_vidx.create_vector_index(
                collection,
                field,
                dim as usize,
                metric,
                100_000, // max_vectors default
                16,      // m default
                200,     // ef_construction default
                50,      // ef_search default
            ) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_create_vector_index(collection, field, dim, metric, options) -> index_name
    // options: { max_vectors: int, m: int, ef_construction: int, ef_search: int }
    let adapter_vidx_opts = adapter.clone();
    engine.register_fn(
        "db_create_vector_index",
        move |collection: &str, field: &str, dim: i64, metric: &str, options: Map| -> Dynamic {
            let max_vectors = options
                .get("max_vectors")
                .and_then(|v| v.as_int().ok())
                .map(|v| v as usize)
                .unwrap_or(100_000);
            let m = options
                .get("m")
                .and_then(|v| v.as_int().ok())
                .map(|v| v as usize)
                .unwrap_or(16);
            let ef_construction = options
                .get("ef_construction")
                .and_then(|v| v.as_int().ok())
                .map(|v| v as usize)
                .unwrap_or(200);
            let ef_search = options
                .get("ef_search")
                .and_then(|v| v.as_int().ok())
                .map(|v| v as usize)
                .unwrap_or(50);

            match adapter_vidx_opts.create_vector_index(
                collection,
                field,
                dim as usize,
                metric,
                max_vectors,
                m,
                ef_construction,
                ef_search,
            ) {
                Ok(name) => Dynamic::from(name),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_list_vector_indexes(collection) -> array of index info
    let adapter_lvidx = adapter.clone();
    engine.register_fn(
        "db_list_vector_indexes",
        move |collection: &str| -> Dynamic {
            match adapter_lvidx.list_vector_indexes(collection) {
                Ok(indexes) => {
                    let result: Vec<Dynamic> =
                        indexes.into_iter().map(|v| json_to_dynamic(&v)).collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_drop_vector_index(collection, index_name) -> bool
    let adapter_dvidx = adapter.clone();
    engine.register_fn(
        "db_drop_vector_index",
        move |collection: &str, index_name: &str| -> Dynamic {
            match adapter_dvidx.drop_vector_index(collection, index_name) {
                Ok(()) => Dynamic::from(true),
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_vector_search(collection, field, query_vector, limit) -> array of {document, score}
    let max_find_documents = limits.max_find_documents;
    let adapter_vsrch = adapter.clone();
    engine.register_fn(
        "db_vector_search",
        move |collection: &str, field: &str, query_vector: rhai::Array, limit: i64| -> Dynamic {
            // Convert Rhai array to Vec<f32>
            let vector: Vec<f32> = query_vector
                .iter()
                .filter_map(|v| v.as_float().ok().map(|f| f as f32))
                .collect();

            if vector.is_empty() {
                return Dynamic::from(
                    "Error: query_vector must be a non-empty array of numbers".to_string(),
                );
            }

            let effective_limit = (limit as usize).min(max_find_documents);

            match adapter_vsrch.vector_search(collection, field, &vector, effective_limit) {
                Ok(results) => {
                    let result: Vec<Dynamic> = results
                        .into_iter()
                        .map(|(doc, score)| {
                            let mut map = Map::new();
                            map.insert("document".into(), json_to_dynamic(&doc));
                            map.insert("score".into(), Dynamic::from(score as f64));
                            Dynamic::from(map)
                        })
                        .collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_vector_search_filter(collection, field, query_vector, filter, limit) -> array of {document, score}
    let adapter_vsrchf = adapter;
    engine.register_fn(
        "db_vector_search_filter",
        move |collection: &str,
              field: &str,
              query_vector: rhai::Array,
              filter: Map,
              limit: i64|
              -> Dynamic {
            // Convert Rhai array to Vec<f32>
            let vector: Vec<f32> = query_vector
                .iter()
                .filter_map(|v| v.as_float().ok().map(|f| f as f32))
                .collect();

            if vector.is_empty() {
                return Dynamic::from(
                    "Error: query_vector must be a non-empty array of numbers".to_string(),
                );
            }

            let filter_json = map_to_json(&filter);
            let effective_limit = (limit as usize).min(max_find_documents);

            match adapter_vsrchf.vector_search_with_filter(
                collection,
                field,
                &vector,
                &filter_json,
                effective_limit,
            ) {
                Ok(results) => {
                    let result: Vec<Dynamic> = results
                        .into_iter()
                        .map(|(doc, score)| {
                            let mut map = Map::new();
                            map.insert("document".into(), json_to_dynamic(&doc));
                            map.insert("score".into(), Dynamic::from(score as f64));
                            Dynamic::from(map)
                        })
                        .collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );
}

// ============================================================
// RAG Operations
// ============================================================

// RRF_K, MAX_INTERNAL_LIMIT, DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD, DEFAULT_EMBEDDING_PROVIDER
// imported from crate::tools::defaults

/// Maximum documents returned by aggregate to prevent OOM
const MAX_AGGREGATE_DOCUMENTS: usize = 10_000;

/// Maximum unique values from distinct to prevent OOM
const MAX_DISTINCT_VALUES: usize = 10_000;

/// Reserved metadata keys that cannot be overwritten by user input
const RESERVED_METADATA_KEYS: &[&str] = &["_id", "doc_id", "chunk_index", "chunk_total"];

/// System collection for RAG configs
const RAG_CONFIG_COLLECTION: &str = "_system.rag";

fn register_rag_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    embedding_manager: Option<Arc<EmbeddingManager>>,
    limits: &ScriptLimits,
) {
    let max_find_documents = limits.max_find_documents;

    // db_hybrid_search(collection, query) -> array of result documents
    let adapter_hs1 = adapter.clone();
    let emb_mgr1 = embedding_manager.clone();
    engine.register_fn(
        "db_hybrid_search",
        move |collection: &str, query: &str| -> Dynamic {
            hybrid_search_impl(
                &adapter_hs1,
                &emb_mgr1,
                collection,
                query,
                None,
                max_find_documents,
            )
        },
    );

    // db_hybrid_search(collection, query, options) -> array of result documents
    let adapter_hs2 = adapter.clone();
    let emb_mgr2 = embedding_manager.clone();
    engine.register_fn(
        "db_hybrid_search",
        move |collection: &str, query: &str, options: Map| -> Dynamic {
            hybrid_search_impl(
                &adapter_hs2,
                &emb_mgr2,
                collection,
                query,
                Some(options),
                max_find_documents,
            )
        },
    );

    // db_rag_import(collection, content, title) -> #{doc_id, chunks_created}
    let adapter_imp1 = adapter.clone();
    let emb_mgr3 = embedding_manager.clone();
    engine.register_fn(
        "db_rag_import",
        move |collection: &str, content: &str, title: &str| -> Dynamic {
            let mut options = Map::new();
            options.insert("title".into(), Dynamic::from(title.to_string()));
            rag_import_impl(&adapter_imp1, &emb_mgr3, collection, content, options)
        },
    );

    // db_rag_import(collection, content, options) -> #{doc_id, chunks_created}
    let adapter_imp2 = adapter.clone();
    let emb_mgr4 = embedding_manager.clone();
    engine.register_fn(
        "db_rag_import",
        move |collection: &str, content: &str, options: Map| -> Dynamic {
            rag_import_impl(&adapter_imp2, &emb_mgr4, collection, content, options)
        },
    );

    // db_rag_create(collection) -> #{success, config}
    let adapter_cr1 = adapter.clone();
    let emb_mgr5 = embedding_manager.clone();
    engine.register_fn("db_rag_create", move |collection: &str| -> Dynamic {
        rag_create_impl(&adapter_cr1, &emb_mgr5, collection, None)
    });

    // db_rag_create(collection, options) -> #{success, config}
    let adapter_cr2 = adapter.clone();
    let emb_mgr6 = embedding_manager.clone();
    engine.register_fn(
        "db_rag_create",
        move |collection: &str, options: Map| -> Dynamic {
            rag_create_impl(&adapter_cr2, &emb_mgr6, collection, Some(options))
        },
    );

    // db_rag_stats(collection) -> #{chunk_count, source_document_count, ...}
    let adapter_st = adapter;
    let emb_mgr7 = embedding_manager;
    engine.register_fn("db_rag_stats", move |collection: &str| -> Dynamic {
        rag_stats_impl(&adapter_st, &emb_mgr7, collection)
    });
}

// ============================================================
// RAG Implementation Functions
// ============================================================

/// Get RAG config for a collection from _system.rag
fn get_rag_config(
    adapter: &IronBaseAdapter,
    collection: &str,
) -> Option<(String, String, String, String)> {
    // (embedding_field, text_field, provider, language)
    let result = adapter.find_one(RAG_CONFIG_COLLECTION, json!({"collection": collection}));

    match result {
        Ok(Some(doc)) => {
            let embedding_field = doc
                .get("embedding_field")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_EMBEDDING_FIELD)
                .to_string();
            let text_field = doc
                .get("text_field")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_TEXT_FIELD)
                .to_string();
            let provider = doc
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_EMBEDDING_PROVIDER)
                .to_string();
            let language = doc
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_string();
            Some((embedding_field, text_field, provider, language))
        }
        _ => None,
    }
}

/// Save RAG config to _system.rag collection
fn save_rag_config(
    adapter: &IronBaseAdapter,
    collection: &str,
    embedding_field: &str,
    text_field: &str,
    provider: &str,
    language: &str,
    dimension: usize,
) -> Result<(), String> {
    // Ensure system collection exists - log errors but continue
    if let Err(e) = adapter.create_collection(RAG_CONFIG_COLLECTION) {
        // Collection may already exist, which is fine
        tracing::debug!("RAG config collection creation: {}", e);
    }

    let config = json!({
        "collection": collection,
        "embedding_field": embedding_field,
        "text_field": text_field,
        "provider": provider,
        "language": language,
        "dimension": dimension,
        "created_at": chrono::Utc::now().to_rfc3339()
    });

    // Try update first, if no match then insert
    let filter = json!({"collection": collection});
    let update_result = adapter
        .update_one(
            RAG_CONFIG_COLLECTION,
            filter,
            json!({"$set": config.clone()}),
        )
        .map_err(|e| format!("Failed to update RAG config: {}", e))?;

    if update_result.modified_count > 0 || update_result.matched_count > 0 {
        return Ok(());
    }

    // No existing config found, insert new one
    adapter
        .insert_one(RAG_CONFIG_COLLECTION, config)
        .map(|_| ())
        .map_err(|e| format!("Failed to insert RAG config: {}", e))
}

/// Hybrid search implementation (RRF fusion with reranking, chunk merge, MMR)
///
/// Delegates to fusion.rs for reranking, chunk merge, and MMR diversity reranking.
/// Consistent with the MCP `hybrid_search` tool (hybrid.rs + fusion.rs).
fn hybrid_search_impl(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    collection: &str,
    query: &str,
    options: Option<Map>,
    max_find_documents: usize,
) -> Dynamic {
    // Validate inputs
    if collection.is_empty() {
        return Dynamic::from("Error: collection name cannot be empty".to_string());
    }
    if query.is_empty() {
        return Dynamic::from("Error: query cannot be empty".to_string());
    }

    // Get embedding manager
    let manager = match embedding_manager {
        Some(m) => m,
        None => {
            return Dynamic::from(
                "Error: Embedding manager not available. Set IRONBASE_FASTTEXT_MODEL.".to_string(),
            )
        }
    };

    // Parse options
    let opts = options.unwrap_or_default();
    let limit = (get_int_option_or(&opts, "limit", 10) as usize).min(max_find_documents);
    let rrf_k = get_float_option_or(&opts, "rrf_k", DEFAULT_RRF_K);
    let rerank = get_bool_option_or(&opts, "rerank", true);
    let deduplicate = get_bool_option_or(&opts, "deduplicate", true);
    let mmr_lambda = get_float_option_or(&opts, "mmr_lambda", 0.5);
    let merge_chunks = get_bool_option_or(&opts, "merge_chunks", true);
    let title_field = get_string_option(&opts, "title_field");
    let mode = get_string_option(&opts, "mode");
    let filter_val = get_json_option(&opts, "filter");
    let search_mode = get_string_option(&opts, "search_mode");
    let text_fields_opt: Option<Vec<String>> = get_string_array_option(&opts, "text_fields");

    // Reject removed parameter with explicit error
    if opts.contains_key("dedup_threshold") {
        return Dynamic::from(
            "Error: Parameter 'dedup_threshold' has been removed. Use 'mmr_lambda' (0.0-1.0) instead."
                .to_string(),
        );
    }

    // Resolve weights: explicit > search_mode preset > balanced default
    let (vector_weight, fulltext_weight) =
        resolve_search_mode_weights(&opts, search_mode.as_deref());

    // Get RAG config or use defaults
    let (embedding_field, text_field, provider_name) = match get_rag_config(adapter, collection) {
        Some((ef, tf, prov, _)) => (ef, tf, prov),
        None => {
            let auto_provider = adapter
                .get_auto_embedding_config(collection)
                .ok()
                .flatten()
                .map(|c| c.provider);
            (
                DEFAULT_EMBEDDING_FIELD.to_string(),
                DEFAULT_TEXT_FIELD.to_string(),
                auto_provider.unwrap_or_else(|| manager.default_provider_name().to_string()),
            )
        }
    };

    // Embed query
    let query_vector = match manager.embed_query(query, Some(&provider_name)) {
        Ok(v) => v,
        Err(e) => return Dynamic::from(format!("Error: Query embedding failed: {}", e)),
    };

    // Internal limit for better fusion coverage (capped for OOM protection)
    let internal_limit = (limit * 3).min(MAX_INTERNAL_LIMIT);

    // ================================================================
    // Vector search (with optional filter)
    // ================================================================
    let vector_results = if let Some(ref filter) = filter_val {
        match adapter.vector_search_with_filter(
            collection,
            &embedding_field,
            &query_vector,
            filter,
            internal_limit,
        ) {
            Ok(r) => r,
            Err(e) => return Dynamic::from(format!("Error: Vector search failed: {}", e)),
        }
    } else {
        match adapter.vector_search(collection, &embedding_field, &query_vector, internal_limit) {
            Ok(r) => r,
            Err(e) => return Dynamic::from(format!("Error: Vector search failed: {}", e)),
        }
    };

    // Build vector rank map (1-indexed)
    let mut vector_ranks: HashMap<String, usize> = HashMap::with_capacity(vector_results.len());
    let mut vector_docs: HashMap<String, (serde_json::Value, f32)> =
        HashMap::with_capacity(vector_results.len());
    for (rank, (doc, score)) in vector_results.into_iter().enumerate() {
        if let Some(id) = doc.get("_id").and_then(fusion_id_to_string) {
            vector_ranks.insert(id.clone(), rank + 1);
            vector_docs.insert(id, (doc, score));
        }
    }

    // ================================================================
    // Fulltext search (with filter + and_mode + multi-field support)
    // ================================================================
    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None,
        filter: filter_val.clone(),
        and_mode: mode.as_deref() == Some("and"),
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
        target_doc_ids: None,
    };

    let text_results = if let Some(ref fields) = text_fields_opt {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        match adapter.fulltext_search_multi(collection, &field_refs, query, text_options) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    "hybrid_search: fulltext multi-field not available: {}. Using vector-only.",
                    e
                );
                Vec::new()
            }
        }
    } else {
        match adapter.fulltext_search(collection, &text_field, query, text_options) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    "hybrid_search: fulltext index not available for '{}.{}': {}. Using vector-only search.",
                    collection, text_field, e
                );
                Vec::new()
            }
        }
    };

    // Build fulltext rank map (1-indexed)
    let mut text_ranks: HashMap<String, usize> = HashMap::with_capacity(text_results.len());
    let mut text_docs: HashMap<String, (serde_json::Value, f64)> =
        HashMap::with_capacity(text_results.len());
    for (rank, res) in text_results.into_iter().enumerate() {
        if let Some(id) = res.document.get("_id").and_then(fusion_id_to_string) {
            text_ranks.insert(id.clone(), rank + 1);
            text_docs.insert(id, (res.document, res.score));
        }
    }

    // ================================================================
    // RRF Fusion (using fusion::FusedResult)
    // ================================================================
    let mut all_ids: HashSet<String> =
        HashSet::with_capacity(vector_ranks.len() + text_ranks.len());
    all_ids.extend(vector_ranks.keys().cloned());
    all_ids.extend(text_ranks.keys().cloned());

    let default_rank = internal_limit + 1;

    let mut fused: Vec<FusedResult> = Vec::with_capacity(all_ids.len());
    for id in all_ids.iter() {
        let v_rank = *vector_ranks.get(id).unwrap_or(&default_rank);
        let t_rank = *text_ranks.get(id).unwrap_or(&default_rank);

        let rrf_score = vector_weight * (1.0 / (rrf_k + v_rank as f64))
            + fulltext_weight * (1.0 / (rrf_k + t_rank as f64));

        let v_score = vector_docs.get(id).map(|(_, s)| *s);
        let t_score = text_docs.get(id).map(|(_, s)| *s);

        let doc = match vector_docs
            .get(id)
            .map(|(d, _)| d.clone())
            .or_else(|| text_docs.get(id).map(|(d, _)| d.clone()))
        {
            Some(d) => d,
            None => continue,
        };

        fused.push(FusedResult {
            id: id.clone(),
            doc,
            rrf_score,
            final_score: rrf_score,
            rerank_boost: 1.0,
            v_rank,
            t_rank,
            v_score,
            t_score,
        });
    }

    // Sort by RRF score
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ================================================================
    // Reranking via fusion::rerank_results
    // ================================================================
    if rerank {
        rerank_results(&mut fused, query, &text_field, title_field.as_deref());
    }

    // ================================================================
    // Adjacent chunk merge via fusion::merge_adjacent_chunks
    // ================================================================
    if merge_chunks {
        merge_adjacent_chunks(&mut fused, &text_field);
    }

    // ================================================================
    // MMR diversity reranking via fusion::mmr_reorder
    // ================================================================
    if deduplicate {
        mmr_reorder(&mut fused, &embedding_field, mmr_lambda, limit);
    } else {
        fused.truncate(limit);
    }

    // ================================================================
    // Response — Dynamic conversion
    // ================================================================
    let results: Vec<Dynamic> = fused
        .into_iter()
        .map(|item| {
            let result = json_to_dynamic(&item.doc);
            if let Some(mut map) = result.clone().try_cast::<Map>() {
                map.insert("_rrf_score".into(), Dynamic::from(item.rrf_score));
                map.insert("_final_score".into(), Dynamic::from(item.final_score));
                map.insert("_rerank_boost".into(), Dynamic::from(item.rerank_boost));
                map.insert("_vector_rank".into(), Dynamic::from(item.v_rank as i64));
                map.insert("_text_rank".into(), Dynamic::from(item.t_rank as i64));
                if let Some(vs) = item.v_score {
                    map.insert("_vector_score".into(), Dynamic::from(vs as f64));
                }
                if let Some(ts) = item.t_score {
                    map.insert("_text_score".into(), Dynamic::from(ts));
                }
                Dynamic::from(map)
            } else {
                result
            }
        })
        .collect();

    Dynamic::from(results)
}

/// RAG import implementation
fn rag_import_impl(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    collection: &str,
    content: &str,
    options: Map,
) -> Dynamic {
    // Validate inputs
    if collection.is_empty() {
        return Dynamic::from("Error: collection name cannot be empty".to_string());
    }
    if content.is_empty() {
        return Dynamic::from("Error: content cannot be empty".to_string());
    }

    // Get embedding manager
    let manager = match embedding_manager {
        Some(m) => m,
        None => {
            return Dynamic::from(
                "Error: Embedding manager not available. Set IRONBASE_FASTTEXT_MODEL.".to_string(),
            )
        }
    };

    // Parse options using helper functions
    let title = get_string_option(&options, "title").unwrap_or_default();
    let doc_id =
        get_string_option(&options, "doc_id").unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let chunk_size = get_int_option_or(&options, "chunk_size", 1000) as usize;
    let overlap = get_int_option_or(&options, "overlap", 100) as usize;
    let mode_str = get_string_option_or(&options, "mode", "auto");

    // Get RAG config or use defaults
    let (embedding_field, text_field, provider_name) = match get_rag_config(adapter, collection) {
        Some((ef, tf, prov, _)) => (ef, tf, prov),
        None => {
            let auto_provider = adapter
                .get_auto_embedding_config(collection)
                .ok()
                .flatten()
                .map(|c| c.provider);
            (
                DEFAULT_EMBEDDING_FIELD.to_string(),
                DEFAULT_TEXT_FIELD.to_string(),
                auto_provider.unwrap_or_else(|| manager.default_provider_name().to_string()),
            )
        }
    };

    // Get provider
    let provider = match manager.get_provider(&provider_name) {
        Some(p) => p,
        None => return Dynamic::from(format!("Error: Provider '{}' not available", provider_name)),
    };

    // Chunk content
    let mode = ChunkMode::parse(&mode_str);
    let chunk_options = ChunkOptions::default()
        .with_chunk_size(chunk_size)
        .with_overlap(overlap)
        .with_mode(mode);

    let chunks = match chunk_content(content, &chunk_options) {
        Ok(c) => c,
        Err(e) => return Dynamic::from(format!("Error: Chunking failed: {}", e)),
    };

    if chunks.is_empty() {
        let mut result = Map::new();
        result.insert("success".into(), Dynamic::from(true));
        result.insert("doc_id".into(), Dynamic::from(doc_id));
        result.insert("chunks_created".into(), Dynamic::from(0_i64));
        result.insert(
            "message".into(),
            Dynamic::from("No chunks generated from content".to_string()),
        );
        return Dynamic::from(result);
    }

    // Generate embeddings
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
    let embeddings = match provider.embed_batch(&texts) {
        Ok(e) => e,
        Err(e) => return Dynamic::from(format!("Error: Embedding failed: {}", e)),
    };

    // Ensure collection exists
    let _ = adapter.create_collection(collection);

    // Ensure indexes exist
    let _ = adapter.create_vector_index(
        collection,
        &embedding_field,
        provider.dimension(),
        "cosine",
        100_000,
        16,
        100,
        50,
    );
    let _ = adapter.create_fulltext_index(collection, &text_field, "none", Some(2), Some(true));

    // Build and insert documents
    let mut documents: Vec<serde_json::Value> = Vec::with_capacity(chunks.len());
    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        let mut doc = json!({
            "doc_id": doc_id,
            "chunk_index": chunk.index,
            "chunk_total": chunk.total,
            "start_char": chunk.start_char,
            "end_char": chunk.end_char
        });

        if let Some(obj) = doc.as_object_mut() {
            obj.insert(text_field.clone(), json!(chunk.text));
            obj.insert(embedding_field.clone(), json!(embedding));
        }

        if !title.is_empty() {
            doc["title"] = json!(title);
        }
        if let Some(ref heading) = chunk.heading {
            doc["section"] = json!(heading);
        }

        // Add custom metadata from options (with security filtering)
        if let Some(metadata) = options.get("metadata") {
            let meta_json = dynamic_to_json(metadata);
            if let Some(meta_obj) = meta_json.as_object() {
                if let Some(doc_obj) = doc.as_object_mut() {
                    for (k, v) in meta_obj {
                        // SECURITY: Skip reserved keys to prevent injection
                        if RESERVED_METADATA_KEYS.contains(&k.as_str()) {
                            tracing::warn!("Ignoring reserved metadata key '{}' in rag_import", k);
                            continue;
                        }
                        // Also skip embedding and text fields
                        if k == &embedding_field || k == &text_field {
                            tracing::warn!(
                                "Ignoring protected field '{}' in rag_import metadata",
                                k
                            );
                            continue;
                        }
                        doc_obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        documents.push(doc);
    }

    // Insert documents
    match adapter.insert_many(collection, documents) {
        Ok(ids) => {
            let mut result = Map::new();
            result.insert("success".into(), Dynamic::from(true));
            result.insert("doc_id".into(), Dynamic::from(doc_id));
            result.insert("chunks_created".into(), Dynamic::from(ids.len() as i64));
            result.insert(
                "dimension".into(),
                Dynamic::from(provider.dimension() as i64),
            );
            result.insert("provider".into(), Dynamic::from(provider_name));
            Dynamic::from(result)
        }
        Err(e) => Dynamic::from(format!("Error: Insert failed: {}", e)),
    }
}

/// RAG create implementation
fn rag_create_impl(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    collection: &str,
    options: Option<Map>,
) -> Dynamic {
    // Validate inputs
    if collection.is_empty() {
        return Dynamic::from("Error: collection name cannot be empty".to_string());
    }

    // Get embedding manager
    let manager = match embedding_manager {
        Some(m) => m,
        None => {
            return Dynamic::from(
                "Error: Embedding manager not available. Set IRONBASE_FASTTEXT_MODEL.".to_string(),
            )
        }
    };

    // Parse options using helper functions
    let opts = options.unwrap_or_default();
    let language = get_string_option_or(&opts, "language", "none");
    let embedding_field = get_string_option_or(&opts, "embedding_field", DEFAULT_EMBEDDING_FIELD);
    let text_field = get_string_option_or(&opts, "text_field", DEFAULT_TEXT_FIELD);
    let provider_name = get_string_option_or(&opts, "provider", manager.default_provider_name());

    // Get provider
    let provider = match manager.get_provider(&provider_name) {
        Some(p) => p,
        None => {
            let available: Vec<_> = manager
                .list_models()
                .iter()
                .map(|m| m.provider.clone())
                .collect();
            return Dynamic::from(format!(
                "Error: Provider '{}' not found. Available: {:?}",
                provider_name, available
            ));
        }
    };

    let dimension = provider.dimension();

    // Create collection
    let collection_created = adapter.create_collection(collection).is_ok();

    // Create vector index
    let vector_created = adapter
        .create_vector_index(
            collection,
            &embedding_field,
            dimension,
            "cosine",
            100_000,
            16,
            100,
            50,
        )
        .is_ok();

    // Create fulltext index
    let fulltext_created = adapter
        .create_fulltext_index(collection, &text_field, &language, Some(2), Some(true))
        .is_ok();

    // Save RAG config
    if let Err(e) = save_rag_config(
        adapter,
        collection,
        &embedding_field,
        &text_field,
        &provider_name,
        &language,
        dimension,
    ) {
        return Dynamic::from(format!("Error: Failed to save RAG config: {}", e));
    }

    let mut result = Map::new();
    result.insert("success".into(), Dynamic::from(true));
    result.insert("collection".into(), Dynamic::from(collection.to_string()));

    let mut config = Map::new();
    config.insert("embedding_field".into(), Dynamic::from(embedding_field));
    config.insert("text_field".into(), Dynamic::from(text_field));
    config.insert("provider".into(), Dynamic::from(provider_name));
    config.insert("language".into(), Dynamic::from(language));
    config.insert("dimension".into(), Dynamic::from(dimension as i64));
    result.insert("config".into(), Dynamic::from(config));

    let mut indexes = Map::new();
    indexes.insert(
        "collection_created".into(),
        Dynamic::from(collection_created),
    );
    indexes.insert("vector_created".into(), Dynamic::from(vector_created));
    indexes.insert("fulltext_created".into(), Dynamic::from(fulltext_created));
    result.insert("indexes".into(), Dynamic::from(indexes));

    Dynamic::from(result)
}

/// RAG stats implementation
fn rag_stats_impl(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    collection: &str,
) -> Dynamic {
    // Validate inputs
    if collection.is_empty() {
        return Dynamic::from("Error: collection name cannot be empty".to_string());
    }

    // Get RAG config
    let rag_config = get_rag_config(adapter, collection);

    // Get document count (chunks)
    let chunk_count = adapter.count_documents(collection, json!({})).unwrap_or(0);

    // Get unique doc_ids (source documents)
    let source_doc_count = adapter
        .distinct(collection, "doc_id", json!({}))
        .map(|v| v.len())
        .unwrap_or(0);

    // Get vector indexes
    let vector_indexes = adapter.list_vector_indexes(collection).unwrap_or_default();

    // Get fulltext indexes
    let fulltext_indexes = adapter
        .list_fulltext_indexes(collection)
        .unwrap_or_default();

    let mut result = Map::new();
    result.insert("collection".into(), Dynamic::from(collection.to_string()));
    result.insert("rag_enabled".into(), Dynamic::from(rag_config.is_some()));

    if let Some((ef, tf, prov, lang)) = rag_config {
        let mut config = Map::new();
        config.insert("embedding_field".into(), Dynamic::from(ef));
        config.insert("text_field".into(), Dynamic::from(tf));
        config.insert("provider".into(), Dynamic::from(prov.clone()));
        config.insert("language".into(), Dynamic::from(lang));
        result.insert("config".into(), Dynamic::from(config));

        // Provider info
        if let Some(ref mgr) = embedding_manager {
            if let Some(provider) = mgr.get_provider(&prov) {
                let mut prov_info = Map::new();
                prov_info.insert("name".into(), Dynamic::from(prov));
                prov_info.insert(
                    "dimension".into(),
                    Dynamic::from(provider.dimension() as i64),
                );
                prov_info.insert(
                    "model".into(),
                    Dynamic::from(provider.model_name().to_string()),
                );
                result.insert("provider_info".into(), Dynamic::from(prov_info));
            }
        }
    }

    let mut stats = Map::new();
    stats.insert("chunk_count".into(), Dynamic::from(chunk_count as i64));
    stats.insert(
        "source_document_count".into(),
        Dynamic::from(source_doc_count as i64),
    );
    stats.insert(
        "vector_index_count".into(),
        Dynamic::from(vector_indexes.len() as i64),
    );
    stats.insert(
        "fulltext_index_count".into(),
        Dynamic::from(fulltext_indexes.len() as i64),
    );
    result.insert("stats".into(), Dynamic::from(stats));

    Dynamic::from(result)
}
