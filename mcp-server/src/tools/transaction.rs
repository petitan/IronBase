//! Transaction management tool handlers

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{get_object, get_string, parse_transaction_id, validate_collection_name};

/// Dispatch transaction tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "begin_transaction" => handle_begin_transaction(adapter),
        "commit_transaction" => handle_commit_transaction(params, adapter),
        "rollback_transaction" => handle_rollback_transaction(params, adapter),
        "insert_one_tx" => handle_insert_one_tx(params, adapter),
        "update_one_tx" => handle_update_one_tx(params, adapter),
        "delete_one_tx" => handle_delete_one_tx(params, adapter),
        "transaction_status" => handle_transaction_status(adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown transaction tool: {}",
            name
        ))),
    }
}

fn handle_begin_transaction(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = adapter.begin_transaction();
    Ok(json!({
        "transaction_id": tx_id.to_string(),
        "message": "Transaction started. Use _tx operations with this ID. Only one write transaction can be active at a time."
    }))
}

fn handle_commit_transaction(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = parse_transaction_id(&params)?;
    adapter.commit_transaction(tx_id)?;
    Ok(json!({
        "success": true,
        "message": "Transaction committed successfully"
    }))
}

fn handle_rollback_transaction(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = parse_transaction_id(&params)?;
    adapter.rollback_transaction(tx_id)?;
    Ok(json!({
        "success": true,
        "message": "Transaction rolled back successfully"
    }))
}

fn handle_insert_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = parse_transaction_id(&params)?;
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let document = get_object(&params, "document")?;
    let id = adapter.insert_one_tx(&collection, document, tx_id)?;
    Ok(json!({"inserted_id": id}))
}

fn handle_update_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = parse_transaction_id(&params)?;
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let update = get_object(&params, "update")?;
    let result = adapter.update_one_tx(&collection, filter, update, tx_id)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_delete_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let tx_id = parse_transaction_id(&params)?;
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let filter = get_object(&params, "filter")?;
    let count = adapter.delete_one_tx(&collection, filter, tx_id)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_transaction_status(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let holder = adapter.get_write_lock_holder();
    Ok(json!({
        "has_active_write_transaction": holder.is_some(),
        "write_lock_holder": holder.map(|id| id.to_string())
    }))
}
