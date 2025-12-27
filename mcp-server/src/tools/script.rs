//! Script management tool handlers

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use crate::scripting::{RhaiEngine, ScriptListFilter, ScriptManager, ScriptOptions};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{get_string, validate_script_name};

/// Dispatch script tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "script_save" => handle_script_save(params, adapter),
        "script_list" => handle_script_list(params, adapter),
        "script_get" => handle_script_get(params, adapter),
        "script_delete" => handle_script_delete(params, adapter),
        "script_run" => handle_script_run(params, adapter),
        "script_exec" => handle_script_exec(params, adapter),
        "script_history" => handle_script_history(params, adapter),
        "script_rollback" => handle_script_rollback(params, adapter),
        "script_version_get" => handle_script_version_get(params, adapter),
        "script_tags_add" => handle_script_tags_add(params, adapter),
        "script_tags_remove" => handle_script_tags_remove(params, adapter),
        "script_stats" => handle_script_stats(params, adapter),
        _ => Err(McpError::InvalidParams(format!(
            "Unknown script tool: {}",
            name
        ))),
    }
}

fn parse_tags(params: &Value) -> Option<Vec<String>> {
    params.get("tags").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    })
}

fn parse_dependencies(params: &Value) -> Option<Vec<String>> {
    params.get("dependencies").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    })
}

fn handle_script_save(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    validate_script_name(&name)?;
    let code = get_string(&params, "code")?;
    let description = params.get("description").and_then(|v| v.as_str());
    let tags = parse_tags(&params);
    let dependencies = parse_dependencies(&params);

    let manager = ScriptManager::new(Arc::clone(adapter));
    let version = manager.save(&name, &code, description, tags, dependencies)?;
    Ok(json!({"success": true, "name": name, "version": version}))
}

fn handle_script_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let manager = ScriptManager::new(Arc::clone(adapter));
    let filter = {
        let tags = parse_tags(&params);
        let match_all = params
            .get("match_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if tags.is_some() {
            Some(ScriptListFilter {
                tags,
                match_all_tags: match_all,
            })
        } else {
            None
        }
    };
    let scripts = manager.list(filter)?;
    Ok(json!({"scripts": scripts, "count": scripts.len()}))
}

fn handle_script_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get(&name)? {
        Some(script) => Ok(json!({
            "name": script.name,
            "code": script.code,
            "description": script.description,
            "created_at": script.created_at,
            "updated_at": script.updated_at,
            "version": script.version,
            "tags": script.tags,
            "dependencies": script.dependencies
        })),
        None => Err(McpError::InvalidParams(format!(
            "Script '{}' not found",
            name
        ))),
    }
}

fn handle_script_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let deleted = manager.delete(&name)?;
    if deleted {
        Ok(json!({"success": true, "deleted": name}))
    } else {
        Err(McpError::InvalidParams(format!(
            "Script '{}' not found",
            name
        )))
    }
}

fn handle_script_run(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let script_params = params.get("params").cloned();
    let options = params
        .get("max_operations")
        .and_then(|v| v.as_u64())
        .map(ScriptOptions::with_max_operations);

    let manager = ScriptManager::new(Arc::clone(adapter));
    let engine = RhaiEngine::new(Arc::clone(adapter));
    let result = manager.run_script_with_options(&name, script_params, &engine, options)?;

    Ok(json!({
        "success": true,
        "result": result.result,
        "logs": result.logs,
        "execution_time_ms": result.execution_time_ms
    }))
}

fn handle_script_exec(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let code = get_string(&params, "code")?;
    let script_params = params.get("params").cloned();
    let options = params
        .get("max_operations")
        .and_then(|v| v.as_u64())
        .map(ScriptOptions::with_max_operations);

    let engine = RhaiEngine::new(Arc::clone(adapter));
    let result = match options {
        Some(opts) => engine.run_with_options(&code, script_params, opts)?,
        None => engine.run(&code, script_params)?,
    };

    Ok(json!({
        "success": true,
        "result": result.result,
        "logs": result.logs,
        "execution_time_ms": result.execution_time_ms
    }))
}

fn handle_script_history(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let manager = ScriptManager::new(Arc::clone(adapter));
    let history = manager.get_history(&name, limit)?;
    Ok(json!({"history": history, "count": history.len()}))
}

fn handle_script_rollback(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let version = params
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| McpError::InvalidParams("version is required".to_string()))?
        as u32;
    let manager = ScriptManager::new(Arc::clone(adapter));
    let new_version = manager.rollback(&name, version)?;
    Ok(json!({"success": true, "name": name, "new_version": new_version}))
}

fn handle_script_version_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let version = params
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| McpError::InvalidParams("version is required".to_string()))?
        as u32;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get_version(&name, version)? {
        Some(v) => Ok(json!({
            "script_name": v.script_name,
            "version": v.version,
            "code": v.code,
            "description": v.description,
            "tags": v.tags,
            "dependencies": v.dependencies,
            "created_at": v.created_at
        })),
        None => Err(McpError::InvalidParams(format!(
            "Version {} of script '{}' not found",
            version, name
        ))),
    }
}

fn handle_script_tags_add(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let tags: Vec<String> = parse_tags(&params)
        .ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    manager.add_tags(&name, tags.clone())?;
    Ok(json!({"success": true, "name": name, "added_tags": tags}))
}

fn handle_script_tags_remove(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let tags: Vec<String> = parse_tags(&params)
        .ok_or_else(|| McpError::InvalidParams("tags array is required".to_string()))?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    manager.remove_tags(&name, tags.clone())?;
    Ok(json!({"success": true, "name": name, "removed_tags": tags}))
}

fn handle_script_stats(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let name = get_string(&params, "name")?;
    let manager = ScriptManager::new(Arc::clone(adapter));
    match manager.get_stats(&name)? {
        Some(stats) => Ok(json!({
            "name": stats.name,
            "execution_count": stats.execution_count,
            "last_run_at": stats.last_run_at,
            "last_run_success": stats.last_run_success,
            "total_execution_time_ms": stats.total_execution_time_ms,
            "avg_execution_time_ms": stats.avg_execution_time_ms
        })),
        None => Err(McpError::InvalidParams(format!(
            "Script '{}' not found",
            name
        ))),
    }
}
