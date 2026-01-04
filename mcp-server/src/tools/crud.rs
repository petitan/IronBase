//! CRUD tool handlers: find, insert, update, delete, count, distinct, aggregate

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use ironbase_core::find_options::apply_projection;
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{
    get_array, get_object, get_string, parse_limit, parse_projection, parse_skip, parse_sort,
    validate_collection_name, MAX_QUERY_LIMIT,
};

/// Dispatch CRUD tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "insert_one" => handle_insert_one(params, adapter),
        "insert_many" => handle_insert_many(params, adapter),
        "find" => handle_find(params, adapter),
        "find_one" => handle_find_one(params, adapter),
        "update_one" => handle_update_one(params, adapter),
        "update_many" => handle_update_many(params, adapter),
        "delete_one" => handle_delete_one(params, adapter),
        "delete_many" => handle_delete_many(params, adapter),
        "count_documents" => handle_count_documents(params, adapter),
        "distinct" => handle_distinct(params, adapter),
        "aggregate" => handle_aggregate(params, adapter),
        _ => Err(McpError::InvalidParams(format!(
            "Unknown CRUD tool: {}",
            name
        ))),
    }
}

fn handle_insert_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let document = get_object(&params, "document")?;
    let id = adapter.insert_one(&collection, document)?;
    Ok(json!({"inserted_id": id}))
}

fn handle_insert_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let documents = get_array(&params, "documents")?;
    let ids = adapter.insert_many(&collection, documents)?;
    Ok(json!({"inserted_ids": ids, "inserted_count": ids.len()}))
}

fn handle_find(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let include_total = params
        .get("include_total")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let projection = parse_projection(&params)?;
    let projection_value = projection.map(|proj| json!(proj));
    let limit = parse_limit(&params).or(Some(MAX_QUERY_LIMIT));
    let options = FindOptions {
        projection: projection_value,
        sort: parse_sort(&params),
        limit,
        skip: parse_skip(&params),
        include_total,
    };
    let result = adapter.find(&collection, query, options)?;
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
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let projection = parse_projection(&params)?;
    let document = adapter.find_one(&collection, query)?;

    // Apply projection if specified
    let result = match (document, projection) {
        (Some(doc), Some(proj)) => Some(
            apply_projection(&doc, &proj).map_err(|e| McpError::InvalidParams(e.to_string()))?,
        ),
        (doc, _) => doc,
    };
    Ok(json!({"document": result}))
}

fn handle_update_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let update = get_object(&params, "update")?;
    let result = adapter.update_one(&collection, filter, update)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_update_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let update = get_object(&params, "update")?;
    let result = adapter.update_many(&collection, filter, update)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_delete_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let count = adapter.delete_one(&collection, filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_delete_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let count = adapter.delete_many(&collection, filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_count_documents(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let count = adapter.count_documents(&collection, query)?;
    Ok(json!({"count": count}))
}

fn handle_distinct(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let field = get_string(&params, "field")?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let values = adapter.distinct(&collection, &field, query)?;
    Ok(json!({"values": values, "count": values.len()}))
}

fn handle_aggregate(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let pipeline = get_array(&params, "pipeline")?;
    let results = adapter.aggregate(&collection, pipeline)?;
    Ok(json!({"results": results, "count": results.len()}))
}
