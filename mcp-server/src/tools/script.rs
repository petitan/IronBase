//! Script management tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use crate::scripting::{RhaiEngine, ScriptListFilter, ScriptManager, ScriptOptions};
use ironbase_mcp_types::responses::{
    ScriptDeleteResponse, ScriptHistoryResponse, ScriptListResponse, ScriptRollbackResponse,
    ScriptRunResponse, ScriptSaveResponse, ScriptTagsAddResponse, ScriptTagsRemoveResponse,
};
use serde_json::Value;
use std::sync::Arc;

use super::helpers::validate_script_name;
use super::params::{
    ParseParams, ScriptExecParams, ScriptHistoryParams, ScriptListParams, ScriptNameParams,
    ScriptRunParams, ScriptSaveParams, ScriptTagsParams, ScriptVersionParams,
};

/// Dispatch script tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "script_save" => handle_script_save(params, adapter),
        "script_list" => handle_script_list(params, adapter),
        "script_get" => handle_script_get(params, adapter),
        "script_delete" => handle_script_delete(params, adapter),
        "script_run" => handle_script_run(params, adapter, embedding_manager),
        "script_exec" => handle_script_exec(params, adapter, embedding_manager),
        "script_history" => handle_script_history(params, adapter),
        "script_rollback" => handle_script_rollback(params, adapter),
        "script_version_get" => handle_script_version_get(params, adapter),
        "script_tags_add" => handle_script_tags_add(params, adapter),
        "script_tags_remove" => handle_script_tags_remove(params, adapter),
        "script_stats" => handle_script_stats(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown script tool: {}",
            name
        ))),
    }
}

fn handle_script_save(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptSaveParams = ScriptSaveParams::parse(params)?;
    validate_script_name(&p.name)?;

    let manager = ScriptManager::new(Arc::clone(adapter));
    let version = manager.save(
        &p.name,
        &p.code,
        p.description.as_deref(),
        p.tags,
        p.dependencies,
    )?;
    Ok(serde_json::to_value(ScriptSaveResponse {
        success: true,
        name: p.name,
        version,
    })?)
}

fn handle_script_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptListParams = ScriptListParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let filter = if p.tags.is_some() {
        Some(ScriptListFilter {
            tags: p.tags,
            match_all_tags: p.match_all,
        })
    } else {
        None
    };
    let scripts = manager.list(filter)?;
    Ok(serde_json::to_value(ScriptListResponse {
        count: scripts.len(),
        scripts,
    })?)
}

fn handle_script_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptNameParams = ScriptNameParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get(&p.name)? {
        Some(script) => Ok(serde_json::to_value(script)?),
        None => Err(McpError::invalid_params(format!(
            "Script '{}' not found",
            p.name
        ))),
    }
}

fn handle_script_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptNameParams = ScriptNameParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let deleted = manager.delete(&p.name)?;
    if deleted {
        Ok(serde_json::to_value(ScriptDeleteResponse {
            success: true,
            deleted: p.name,
        })?)
    } else {
        Err(McpError::invalid_params(format!(
            "Script '{}' not found",
            p.name
        )))
    }
}

fn handle_script_run(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: ScriptRunParams = ScriptRunParams::parse(params)?;
    let options = p.max_operations.map(ScriptOptions::with_max_operations);

    let manager = ScriptManager::new(Arc::clone(adapter));
    let engine = RhaiEngine::new(Arc::clone(adapter), embedding_manager.clone());
    let result = manager.run_script_with_options(&p.name, p.params, &engine, options)?;

    Ok(serde_json::to_value(ScriptRunResponse {
        success: true,
        run: result,
    })?)
}

fn handle_script_exec(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: ScriptExecParams = ScriptExecParams::parse(params)?;
    let options = p.max_operations.map(ScriptOptions::with_max_operations);

    let engine = RhaiEngine::new(Arc::clone(adapter), embedding_manager.clone());
    let result = match options {
        Some(opts) => engine.run_with_options(&p.code, p.params, opts)?,
        None => engine.run(&p.code, p.params)?,
    };

    Ok(serde_json::to_value(ScriptRunResponse {
        success: true,
        run: result,
    })?)
}

fn handle_script_history(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptHistoryParams = ScriptHistoryParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let history = manager.get_history(&p.name, p.limit)?;
    Ok(serde_json::to_value(ScriptHistoryResponse {
        count: history.len(),
        history,
    })?)
}

fn handle_script_rollback(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptVersionParams = ScriptVersionParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let new_version = manager.rollback(&p.name, p.version)?;
    Ok(serde_json::to_value(ScriptRollbackResponse {
        success: true,
        name: p.name,
        new_version,
    })?)
}

fn handle_script_version_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptVersionParams = ScriptVersionParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get_version(&p.name, p.version)? {
        Some(v) => Ok(serde_json::to_value(v)?),
        None => Err(McpError::invalid_params(format!(
            "Version {} of script '{}' not found",
            p.version, p.name
        ))),
    }
}

fn handle_script_tags_add(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptTagsParams = ScriptTagsParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    manager.add_tags(&p.name, p.tags.clone())?;
    Ok(serde_json::to_value(ScriptTagsAddResponse {
        success: true,
        name: p.name,
        added_tags: p.tags,
    })?)
}

fn handle_script_tags_remove(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptTagsParams = ScriptTagsParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    manager.remove_tags(&p.name, p.tags.clone())?;
    Ok(serde_json::to_value(ScriptTagsRemoveResponse {
        success: true,
        name: p.name,
        removed_tags: p.tags,
    })?)
}

fn handle_script_stats(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ScriptNameParams = ScriptNameParams::parse(params)?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get_stats(&p.name)? {
        Some(stats) => Ok(serde_json::to_value(stats)?),
        None => Err(McpError::invalid_params(format!(
            "Script '{}' not found",
            p.name
        ))),
    }
}
