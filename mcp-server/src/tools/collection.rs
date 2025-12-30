//! Collection management tool handlers

use crate::acl::SYSTEM_ACL_COLLECTION;
use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{get_string, validate_collection_name};

/// Dispatch collection tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "collection_list" => handle_collection_list(adapter),
        "collection_create" => handle_collection_create(params, adapter),
        "collection_drop" => handle_collection_drop(params, adapter),
        "schema_set" => handle_schema_set(params, adapter),
        "schema_get" => handle_schema_get(params, adapter),
        _ => Err(McpError::InvalidParams(format!(
            "Unknown collection tool: {}",
            name
        ))),
    }
}

fn handle_collection_list(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collections = adapter.list_collections();
    Ok(json!({"collections": collections}))
}

fn handle_collection_create(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    adapter.create_collection(&name)?;
    Ok(json!({"success": true, "collection": name}))
}

fn handle_collection_drop(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    adapter.drop_collection(&name)?;

    // Also delete ACL for this collection
    let acl_deleted = adapter
        .delete_one(SYSTEM_ACL_COLLECTION, json!({"collection": name}))
        .unwrap_or(0)
        > 0;

    Ok(json!({
        "success": true,
        "dropped": name,
        "acl_deleted": acl_deleted
    }))
}

fn handle_schema_set(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let schema = params.get("schema").cloned().filter(|v| !v.is_null());
    adapter.set_schema(&collection, schema.clone())?;
    Ok(json!({"success": true, "schema_set": schema.is_some()}))
}

fn handle_schema_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let schema = adapter.get_schema(&collection)?;
    Ok(json!({"schema": schema}))
}
