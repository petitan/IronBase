//! Collection management tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::acl::SYSTEM_ACL_COLLECTION;
use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::validate_collection_name;
use super::params::{
    CollectionCreateParams, CollectionDropParams, ParseParams, SchemaGetParams, SchemaSetParams,
};

/// Dispatch collection tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "collection_list" => handle_collection_list(adapter),
        "collection_create" => handle_collection_create(params, adapter),
        "collection_drop" => handle_collection_drop(params, adapter),
        "schema_set" => handle_schema_set(params, adapter),
        "schema_get" => handle_schema_get(params, adapter),
        _ => Err(McpError::invalid_params(format!(
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
    let p: CollectionCreateParams = CollectionCreateParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    adapter.create_collection(&p.collection)?;
    Ok(json!({"success": true, "collection": p.collection}))
}

fn handle_collection_drop(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: CollectionDropParams = CollectionDropParams::parse(params)?;
    adapter.drop_collection(&p.collection)?;

    // Also delete ACL for this collection
    let acl_deleted = adapter
        .delete_one(SYSTEM_ACL_COLLECTION, json!({"collection": &p.collection}))
        .unwrap_or(0)
        > 0;

    Ok(json!({
        "success": true,
        "dropped": p.collection,
        "acl_deleted": acl_deleted
    }))
}

fn handle_schema_set(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: SchemaSetParams = SchemaSetParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    let schema = p.schema.filter(|v| !v.is_null());
    adapter.set_schema(&p.collection, schema.clone())?;
    Ok(json!({"success": true, "schema_set": schema.is_some()}))
}

fn handle_schema_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: SchemaGetParams = SchemaGetParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    let schema = adapter.get_schema(&p.collection)?;
    Ok(json!({"schema": schema}))
}
