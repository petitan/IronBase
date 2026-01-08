//! CRUD tool handlers: find, insert, update, delete, count, distinct, aggregate
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use crate::scripting::ScriptLimits;
use ironbase_core::find_options::apply_projection;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::helpers::{
    check_cancelled, validate_collection_name, validate_document, validate_filter, validate_update,
    DEFAULT_QUERY_LIMIT,
};
use super::params::{
    AggregateParams, CountParams, DeleteParams, DistinctParams, FindOneParams, FindParams,
    InsertManyParams, InsertOneParams, ParseParams, UpdateParams,
};

/// Parse sort specification from Value to Vec<(String, i32)>
/// Supports both array format [["field", 1]] and object format {"field": 1}
fn parse_sort_value(sort: Option<Value>) -> Result<Option<Vec<(String, i32)>>> {
    match sort {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            // Array format: [["field", 1], ["field2", -1]]
            let mut result = Vec::new();
            for item in arr {
                let pair = item.as_array().ok_or_else(|| {
                    McpError::invalid_params("Sort array items must be [field, direction] pairs")
                })?;
                if pair.len() != 2 {
                    return Err(McpError::invalid_params(
                        "Sort array items must have exactly 2 elements: [field, direction]",
                    ));
                }
                let field = pair[0]
                    .as_str()
                    .ok_or_else(|| McpError::invalid_params("Sort field name must be a string"))?;
                let direction = pair[1]
                    .as_i64()
                    .ok_or_else(|| McpError::invalid_params("Sort direction must be 1 or -1"))?;
                if direction != 1 && direction != -1 {
                    return Err(McpError::invalid_params(format!(
                        "Sort direction for '{}' must be 1 or -1, got {}",
                        field, direction
                    )));
                }
                result.push((field.to_string(), direction as i32));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(Value::Object(map)) => {
            // Object format: {"field": 1, "field2": -1}
            let mut result = Vec::new();
            for (key, value) in map {
                let direction = value.as_i64().unwrap_or(1) as i32;
                result.push((key, direction));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(_) => Err(McpError::invalid_params(
            "Sort must be an array [[\"field\", 1]] or object {\"field\": 1}",
        )),
    }
}

/// Dispatch CRUD tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<Value> {
    match name {
        "insert_one" => handle_insert_one(params, adapter),
        "insert_many" => handle_insert_many(params, adapter),
        "find" => handle_find(params, adapter, limits, cancel_flag),
        "find_one" => handle_find_one(params, adapter),
        "update_one" => handle_update_one(params, adapter),
        "update_many" => handle_update_many(params, adapter),
        "delete_one" => handle_delete_one(params, adapter),
        "delete_many" => handle_delete_many(params, adapter),
        "count_documents" => handle_count_documents(params, adapter),
        "distinct" => handle_distinct(params, adapter),
        "aggregate" => handle_aggregate(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown CRUD tool: {}",
            name
        ))),
    }
}

fn handle_insert_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: InsertOneParams = InsertOneParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_document(&p.document)?;

    let id = adapter.insert_one(&p.collection, p.document)?;
    Ok(json!({"inserted_id": id}))
}

fn handle_insert_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: InsertManyParams = InsertManyParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let ids = adapter.insert_many(&p.collection, p.documents)?;
    Ok(json!({"inserted_ids": ids, "inserted_count": ids.len()}))
}

fn handle_find(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<Value> {
    // Check for cancellation before starting potentially slow operation
    check_cancelled()?;

    let p: FindParams = FindParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Use dynamic limit from ScriptLimits if available, otherwise default
    let max_limit = limits
        .map(|l| l.max_find_documents)
        .unwrap_or(DEFAULT_QUERY_LIMIT);

    // Apply limit: user's limit capped at max_limit, or max_limit if not specified
    let effective_limit = p.limit.map(|l| l.min(max_limit)).or(Some(max_limit));

    // Get max_result_size from ScriptLimits for OOM protection
    let max_response_bytes = limits.map(|l| l.max_result_size);

    let options = FindOptions {
        projection: p.projection,
        sort: parse_sort_value(p.sort)?,
        limit: effective_limit,
        skip: p.skip,
        include_total: p.include_total,
        max_response_bytes,
        cancel_flag,
    };

    let result = adapter.find(&p.collection, p.query, options)?;
    let mut response = json!({
        "documents": result.documents,
        "count": result.documents.len()
    });
    if let Some(total) = result.total {
        response["total"] = json!(total);
    }
    Ok(response)
}

fn handle_find_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: FindOneParams = FindOneParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let document = adapter.find_one(&p.collection, p.query)?;

    // Apply projection if specified
    let result = match (document, p.projection) {
        (Some(doc), Some(proj)) => {
            // Parse projection to HashMap
            let proj_map: std::collections::HashMap<String, i32> = serde_json::from_value(proj)
                .map_err(|e| {
                    McpError::invalid_params(format!("Invalid projection format: {}", e))
                })?;
            Some(
                apply_projection(&doc, &proj_map)
                    .map_err(|e| McpError::invalid_params(e.to_string()))?,
            )
        }
        (doc, _) => doc,
    };
    Ok(json!({"document": result}))
}

fn handle_update_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: UpdateParams = UpdateParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    let result = adapter.update_one(&p.collection, p.filter, p.update)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_update_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: UpdateParams = UpdateParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    let result = adapter.update_many(&p.collection, p.filter, p.update)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_delete_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DeleteParams = DeleteParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;

    let count = adapter.delete_one(&p.collection, p.filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_delete_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DeleteParams = DeleteParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;

    let count = adapter.delete_many(&p.collection, p.filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_count_documents(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: CountParams = CountParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let count = adapter.count_documents(&p.collection, p.query)?;
    Ok(json!({"count": count}))
}

fn handle_distinct(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DistinctParams = DistinctParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let values = adapter.distinct(&p.collection, &p.field, p.query)?;
    Ok(json!({"values": values, "count": values.len()}))
}

fn handle_aggregate(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: AggregateParams = AggregateParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let results = adapter.aggregate(&p.collection, p.pipeline)?;
    Ok(json!({"results": results, "count": results.len()}))
}
