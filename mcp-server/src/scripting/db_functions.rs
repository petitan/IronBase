//! Database function registrations for Rhai scripts.
//!
//! Provides database operations accessible from scripts:
//! - CRUD operations (find, insert, update, delete)
//! - Aggregation
//! - Index management
//! - Fuzzy and fulltext search
//!
//! All functions include safety limits to prevent OOM attacks.

use crate::adapter::{FindOptions as AdapterFindOptions, FulltextSearchOptions, IronBaseAdapter};
use rhai::{Dynamic, Engine, Map};
use std::collections::HashMap;
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
/// # Arguments
///
/// * `engine` - The Rhai engine to register functions into
/// * `adapter` - Database adapter for executing operations
/// * `limits` - Resource limits for query operations
///
/// # Security
///
/// All write operations to `_system.*` collections are blocked.
/// Query limits are enforced to prevent OOM attacks.
pub fn register_db_functions(
    engine: &mut Engine,
    adapter: Arc<IronBaseAdapter>,
    limits: &ScriptLimits,
) {
    register_read_functions(engine, adapter.clone(), limits);
    register_write_functions(engine, adapter.clone());
    register_index_functions(engine, adapter.clone());
    register_search_functions(engine, adapter, limits);
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
    let adapter_agg = adapter.clone();
    engine.register_fn(
        "db_aggregate",
        move |collection: &str, pipeline: rhai::Array| -> Dynamic {
            let pipeline_vec: Vec<serde_json::Value> =
                pipeline.iter().map(dynamic_to_json).collect();
            match adapter_agg.aggregate(collection, pipeline_vec) {
                Ok(docs) => {
                    let result: Vec<Dynamic> =
                        docs.into_iter().map(|d| json_to_dynamic(&d)).collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e)),
            }
        },
    );

    // db_distinct(collection, field, query) -> array of unique values
    let adapter_dist = adapter;
    engine.register_fn(
        "db_distinct",
        move |collection: &str, field: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_dist.distinct(collection, field, query_json) {
                Ok(values) => {
                    let result: Vec<Dynamic> =
                        values.into_iter().map(|v| json_to_dynamic(&v)).collect();
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

    // db_list_indexes(collection) -> array of index names
    let adapter_lidx = adapter.clone();
    engine.register_fn("db_list_indexes", move |collection: &str| -> Dynamic {
        match adapter_lidx.list_indexes(collection) {
            Ok(indexes) => {
                let result: Vec<Dynamic> = indexes.into_iter().map(Dynamic::from).collect();
                Dynamic::from(result)
            }
            Err(e) => Dynamic::from(format!("Error: {}", e)),
        }
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
                highlight,
                highlight_context,
                highlight_max_snippets,
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
                                        hl_map.insert("field".into(), Dynamic::from(hl.field.clone()));
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
