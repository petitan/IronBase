//! Transaction management tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{parse_transaction_id_str, validate_collection_name};
use super::params::{
    ParseParams, TransactionDeleteParams, TransactionIdParams, TransactionInsertParams,
    TransactionUpdateParams,
};

/// Dispatch transaction tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "transaction_begin" => handle_begin_transaction(adapter),
        "transaction_commit" => handle_commit_transaction(params, adapter),
        "transaction_rollback" => handle_rollback_transaction(params, adapter),
        "transaction_insert_one" => handle_insert_one_tx(params, adapter),
        "transaction_update_one" => handle_update_one_tx(params, adapter),
        "transaction_delete_one" => handle_delete_one_tx(params, adapter),
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
    let p: TransactionIdParams = TransactionIdParams::parse(params)?;
    let tx_id = parse_transaction_id_str(&p.transaction_id)?;
    adapter.commit_transaction(tx_id)?;
    Ok(json!({
        "success": true,
        "message": "Transaction committed successfully"
    }))
}

fn handle_rollback_transaction(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: TransactionIdParams = TransactionIdParams::parse(params)?;
    let tx_id = parse_transaction_id_str(&p.transaction_id)?;
    adapter.rollback_transaction(tx_id)?;
    Ok(json!({
        "success": true,
        "message": "Transaction rolled back successfully"
    }))
}

fn handle_insert_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: TransactionInsertParams = TransactionInsertParams::parse(params)?;
    let tx_id = parse_transaction_id_str(&p.transaction_id)?;
    validate_collection_name(&p.collection)?;

    if !p.document.is_object() {
        return Err(McpError::invalid_params("document must be a JSON object"));
    }

    let id = adapter.insert_one_tx(&p.collection, p.document, tx_id)?;
    Ok(json!({"inserted_id": id}))
}

fn handle_update_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: TransactionUpdateParams = TransactionUpdateParams::parse(params)?;
    let tx_id = parse_transaction_id_str(&p.transaction_id)?;
    validate_collection_name(&p.collection)?;

    if !p.filter.is_object() {
        return Err(McpError::invalid_params("filter must be a JSON object"));
    }
    if !p.update.is_object() {
        return Err(McpError::invalid_params("update must be a JSON object"));
    }

    let result = adapter.update_one_tx(&p.collection, p.filter, p.update, tx_id)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
}

fn handle_delete_one_tx(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: TransactionDeleteParams = TransactionDeleteParams::parse(params)?;
    let tx_id = parse_transaction_id_str(&p.transaction_id)?;
    validate_collection_name(&p.collection)?;

    if !p.filter.is_object() {
        return Err(McpError::invalid_params("filter must be a JSON object"));
    }

    let count = adapter.delete_one_tx(&p.collection, p.filter, tx_id)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_transaction_status(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let holder = adapter.get_write_lock_holder();
    Ok(json!({
        "has_active_write_transaction": holder.is_some(),
        "write_lock_holder": holder.map(|id| id.to_string())
    }))
}
